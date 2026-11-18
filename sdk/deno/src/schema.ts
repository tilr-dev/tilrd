// The codec every generated message has (`gen/tilr/<ns>/v1.ts`), over protobuf-es's
// static schemas (`gen/pb/`): no code is built at run time, so the library runs in a
// page under a strict Content-Security-Policy (no `unsafe-eval`).

import {
  create,
  type DescMessage,
  fromBinary,
  type MessageInitShape,
  type MessageShape,
  toBinary,
} from "@bufbuild/protobuf";

/**
 * Encodes and decodes one message type of the bus schemas. Messages are protobuf-es's:
 * camelCase fields, `bigint` for 64-bit integers (`InputRequest.seq`, `PingRequest.nonce`
 * keep every bit), a oneof as `{ case: "name", value }`, every field set when decoded.
 */
export interface MessageCodec<Desc extends DescMessage> {
  /** The full protobuf name, `tilr.pane.v1.CreateRequest`. */
  readonly name: string;
  /** protobuf-es's schema (`create`, `toBinary`, `fromBinary`, `toJson` take it). */
  readonly schema: Desc;
  /** A message, from a plain object of some of its fields. */
  create(init?: MessageInitShape<Desc>): MessageShape<Desc>;
  encode(message?: MessageInitShape<Desc>): Uint8Array;
  /** A payload that does not decode throws. */
  decode(bytes: Uint8Array): MessageShape<Desc>;
}

/** The codec of the message `schema` describes. */
export function codec<Desc extends DescMessage>(schema: Desc): MessageCodec<Desc> {
  return {
    name: schema.typeName,
    schema,
    create: (init) => create(schema, init),
    encode: (message) => toBinary(schema, create(schema, message)),
    decode: (bytes) => fromBinary(schema, bytes),
  };
}
