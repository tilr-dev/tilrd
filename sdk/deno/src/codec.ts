// Framing of envelopes (design B3; `crates/tilr-bus/src/codec.rs` is the reference).
//
// - WebSocket: one binary message is a flag byte (0 plain, 1 zstd) and one envelope.
//   This library reads both and writes plain: a browser has no zstd compressor, and
//   the broker reads either.
// - Stream sockets (an instance's descriptor 3, its instance socket): a u32 big-endian
//   length and one envelope, never compressed.
//
// An envelope is at most MAX_FRAME bytes after decompression, both ways.

import { decompress } from "fzstd";
import protobuf from "protobufjs/light.js";
import { codec, root } from "./schema.ts";

/** Largest envelope after decompression (`HelloAck.max_frame`). */
export const MAX_FRAME = 16 << 20;
export const FLAG_PLAIN = 0;
export const FLAG_ZSTD = 1;
/** `Chunk.contentEncoding` of a payload its producer compressed with zstd. */
export const ENCODING_ZSTD = "zstd";

/** One envelope, as decoded: `msg` names the variant set (`"call"`, `"reply"`, ...). */
// deno-lint-ignore no-explicit-any
export type Envelope = { msg?: string } & Record<string, any>;

export const EnvelopeCodec = codec<Envelope>("tilr.bus.v1.Envelope");

/** Why an envelope could not be framed or read. */
export class CodecError extends Error {
  constructor(message: string, readonly tooLarge = false) {
    super(message);
    this.name = "CodecError";
  }
}

const EnvelopeType = root.lookupType("tilr.bus.v1.Envelope");

/**
 * Encode an envelope after `prefix` bytes (the flag byte, or the length) into one
 * buffer, so a chunk's payload is copied once.
 */
function encodeAfter(envelope: Envelope, prefix: 1 | 4, maxFrame: number): Uint8Array {
  const writer = protobuf.Writer.create();
  if (prefix === 1) writer.uint32(FLAG_PLAIN); // one byte, 0
  else writer.fixed32(0); // four bytes, overwritten with the length
  const out = EnvelopeType.encode(EnvelopeType.fromObject(envelope), writer).finish();
  const len = out.length - prefix;
  if (len > maxFrame) {
    throw new CodecError(`an envelope of ${len} bytes is over the limit of ${maxFrame}`, true);
  }
  if (prefix === 4) new DataView(out.buffer, out.byteOffset, 4).setUint32(0, len);
  return out;
}

/** Frame an envelope as one binary WebSocket message (plain). */
export function encodeWs(envelope: Envelope, maxFrame = MAX_FRAME): Uint8Array {
  return encodeAfter(envelope, 1, maxFrame);
}

/** Read one binary WebSocket message. */
export function decodeWs(message: Uint8Array): Envelope {
  if (message.length === 0) throw new CodecError("empty packet");
  const flag = message[0];
  let body = message.subarray(1);
  if (flag === FLAG_ZSTD) {
    try {
      body = decompress(body);
    } catch (err) {
      throw new CodecError(`zstd: ${(err as Error).message}`);
    }
  } else if (flag !== FLAG_PLAIN) {
    throw new CodecError(`unknown packet flag ${flag}`);
  }
  if (body.length > MAX_FRAME) {
    throw new CodecError(
      `envelope of ${body.length} bytes is over the limit of ${MAX_FRAME}`,
      true,
    );
  }
  try {
    return EnvelopeCodec.decode(body);
  } catch (err) {
    throw new CodecError(`protobuf: ${(err as Error).message}`);
  }
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
    const frame = this.#take(4 + len).subarray(4);
    try {
      return EnvelopeCodec.decode(frame);
    } catch (err) {
      throw new CodecError(`protobuf: ${(err as Error).message}`);
    }
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
  const plain = decompress(payload);
  if (plain.length > MAX_FRAME) throw new CodecError("chunk is over 16 MiB", true);
  return plain;
}
