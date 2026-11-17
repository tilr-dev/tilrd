// Framing of envelopes (design B3; `crates/tilr-bus/src/codec.rs` is the reference).
//
// - WebSocket: one binary message is a flag byte (0 plain, 1 zstd) and one envelope.
//   This library reads both and writes plain: a browser has no zstd compressor, and
//   the broker reads either.
// - Stream sockets (an instance's descriptor 3, its instance socket): a u32 big-endian
//   length and one envelope, never compressed.
//
// An envelope is at most MAX_FRAME bytes after decompression, both ways.

import { Decompress } from "fzstd";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { EnvelopeSchema } from "../gen/pb/bus_pb.ts";

/** Largest envelope after decompression (`HelloAck.max_frame`). */
export const MAX_FRAME = 16 << 20;
export const FLAG_PLAIN = 0;
export const FLAG_ZSTD = 1;
/** `Chunk.contentEncoding` of a payload its producer compressed with zstd. */
export const ENCODING_ZSTD = "zstd";

/**
 * One envelope as this library handles it: `msg` names the variant set (`"call"`,
 * `"reply"`, ...) and the field of that name holds it, as protobuf-es decodes it
 * (64-bit ids are `bigint`). A `reply`'s oneof is flat too: `result` is `"payload"` or
 * `"error"`, and `payload` or `error` is set.
 */
// deno-lint-ignore no-explicit-any
export type Envelope = { msg?: string } & Record<string, any>;

/** Why an envelope could not be framed or read. */
export class CodecError extends Error {
  constructor(message: string, readonly tooLarge = false) {
    super(message);
    this.name = "CodecError";
  }
}

/** The protobuf-es message of an envelope (`msg: { case, value }`). */
function toMessage(envelope: Envelope) {
  const which = envelope.msg;
  if (!which) return create(EnvelopeSchema);
  let value = envelope[which] ?? {};
  if (which === "reply") {
    const { result: _result, payload, error, ...rest } = value;
    value = {
      ...rest,
      result: error !== undefined
        ? { case: "error", value: error }
        : payload !== undefined
        ? { case: "payload", value: payload }
        : { case: undefined },
    };
  }
  // deno-lint-ignore no-explicit-any
  return create(EnvelopeSchema, { msg: { case: which, value } as any });
}

/** An envelope as this library handles it, from its encoding. */
export function decodeEnvelope(body: Uint8Array): Envelope {
  let message;
  try {
    message = fromBinary(EnvelopeSchema, body);
  } catch (err) {
    throw new CodecError(`protobuf: ${(err as Error).message}`);
  }
  const which = message.msg.case;
  if (!which) return {};
  // deno-lint-ignore no-explicit-any
  let value: any = message.msg.value;
  if (which === "reply") {
    const { result, ...rest } = value;
    value = { ...rest, result: result.case };
    if (result.case) value[result.case] = result.value;
  }
  return { msg: which, [which]: value };
}

/** An envelope's encoding, after `prefix` bytes (the flag byte, or the length). */
function encodeAfter(envelope: Envelope, prefix: 1 | 4, maxFrame: number): Uint8Array {
  let body: Uint8Array;
  try {
    body = toBinary(EnvelopeSchema, toMessage(envelope));
  } catch (err) {
    throw new CodecError(`protobuf: ${(err as Error).message}`);
  }
  if (body.length > maxFrame) {
    throw new CodecError(
      `an envelope of ${body.length} bytes is over the limit of ${maxFrame}`,
      true,
    );
  }
  const out = new Uint8Array(prefix + body.length);
  if (prefix === 1) out[0] = FLAG_PLAIN;
  else new DataView(out.buffer).setUint32(0, body.length);
  out.set(body, prefix);
  return out;
}

/** Frame an envelope as one binary WebSocket message (plain). */
export function encodeWs(envelope: Envelope, maxFrame = MAX_FRAME): Uint8Array {
  return encodeAfter(envelope, 1, maxFrame);
}

/**
 * The size a zstd frame declares (its content size, else its window), checked against
 * `limit` before anything is decompressed: a frame that claims more is refused
 * without allocating for it.
 */
function checkZstdHeader(data: Uint8Array, limit: number): void {
  if (data.length < 5) return; // too short for a header: the decompressor says why
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (view.getUint32(0, true) !== 0xfd2fb528) return;
  const descriptor = data[4];
  const fcsFlag = descriptor >> 6;
  const singleSegment = (descriptor >> 5) & 1;
  const didSize = [0, 1, 2, 4][descriptor & 3];
  let at = 5;
  let declared: number | undefined;
  if (!singleSegment) {
    const window = data[at++];
    if (window === undefined) return;
    const log = 10 + (window >> 3);
    const base = 2 ** log;
    declared = base + (base / 8) * (window & 7);
  }
  at += didSize;
  const fcsSize = fcsFlag === 0 ? (singleSegment ? 1 : 0) : [0, 2, 4, 8][fcsFlag];
  if (fcsSize > 0 && at + fcsSize <= data.length) {
    if (fcsSize === 1) declared = data[at];
    else if (fcsSize === 2) declared = view.getUint16(at, true) + 256;
    else if (fcsSize === 4) declared = view.getUint32(at, true);
    else declared = Number(view.getBigUint64(at, true));
  }
  if (declared !== undefined && declared > limit) {
    throw new CodecError(`a zstd frame of ${declared} bytes is over the limit of ${limit}`, true);
  }
}

/** Decompress zstd data, failing as soon as it comes to more than `limit` bytes. */
export function decompressBounded(data: Uint8Array, limit = MAX_FRAME): Uint8Array {
  checkZstdHeader(data, limit);
  const parts: Uint8Array[] = [];
  let size = 0;
  const stream = new Decompress((chunk) => {
    size += chunk.length;
    if (size > limit) {
      throw new CodecError(`zstd data of more than ${limit} bytes is over the limit`, true);
    }
    parts.push(chunk.slice());
  });
  try {
    stream.push(data, true);
  } catch (err) {
    if (err instanceof CodecError) throw err;
    throw new CodecError(`zstd: ${(err as Error).message}`);
  }
  if (parts.length === 1) return parts[0];
  const out = new Uint8Array(size);
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

/** Read one binary WebSocket message. */
export function decodeWs(message: Uint8Array): Envelope {
  if (message.length === 0) throw new CodecError("empty packet");
  const flag = message[0];
  let body = message.subarray(1);
  if (flag === FLAG_ZSTD) {
    body = decompressBounded(body, MAX_FRAME);
  } else if (flag !== FLAG_PLAIN) {
    throw new CodecError(`unknown packet flag ${flag}`);
  }
  if (body.length > MAX_FRAME) {
    throw new CodecError(
      `envelope of ${body.length} bytes is over the limit of ${MAX_FRAME}`,
      true,
    );
  }
  return decodeEnvelope(body);
}

/** Frame an envelope for a stream socket: u32 big-endian length, then the envelope. */
export function encodeStream(envelope: Envelope, maxFrame = MAX_FRAME): Uint8Array {
  return encodeAfter(envelope, 4, maxFrame);
}

/** Takes envelopes off the bytes read from a stream socket, as they come. */
export class StreamDecoder {
  #parts: Uint8Array[] = [];
  #length = 0;

  push(bytes: Uint8Array): void {
    if (bytes.length === 0) return;
    this.#parts.push(bytes);
    this.#length += bytes.length;
  }

  /** The next whole envelope, `undefined` until one is in; a bad frame throws. */
  next(): Envelope | undefined {
    if (this.#length < 4) return undefined;
    const prefix = this.#peek(4);
    const len = new DataView(prefix.buffer, prefix.byteOffset, 4).getUint32(0);
    if (len > MAX_FRAME) {
      throw new CodecError(`envelope of ${len} bytes is over the limit of ${MAX_FRAME}`, true);
    }
    if (this.#length < 4 + len) return undefined;
    return decodeEnvelope(this.#take(4 + len).subarray(4));
  }

  #peek(n: number): Uint8Array {
    if (this.#parts[0].length >= n) return this.#parts[0].subarray(0, n);
    const out = new Uint8Array(n);
    let at = 0;
    for (const part of this.#parts) {
      const take = Math.min(part.length, n - at);
      out.set(part.subarray(0, take), at);
      at += take;
      if (at === n) break;
    }
    return out;
  }

  #take(n: number): Uint8Array {
    const first = this.#parts[0];
    let out: Uint8Array;
    if (first.length >= n) {
      out = first.subarray(0, n);
      if (first.length === n) this.#parts.shift();
      else this.#parts[0] = first.subarray(n);
    } else {
      out = new Uint8Array(n);
      let at = 0;
      while (at < n) {
        const part = this.#parts[0];
        const take = Math.min(part.length, n - at);
        out.set(part.subarray(0, take), at);
        at += take;
        if (take === part.length) this.#parts.shift();
        else this.#parts[0] = part.subarray(take);
      }
    }
    this.#length -= n;
    return out;
  }
}

/** A chunk's payload, decompressed when its producer compressed it. */
export function chunkBytes(payload: Uint8Array, contentEncoding: string): Uint8Array {
  if (!contentEncoding) return payload;
  if (contentEncoding !== ENCODING_ZSTD) {
    throw new CodecError(
      `chunk has an unknown content encoding ${JSON.stringify(contentEncoding)}`,
    );
  }
  return decompressBounded(payload, MAX_FRAME);
}
