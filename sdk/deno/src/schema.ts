// The bus schemas loaded into protobufjs, and the codec every generated message
// has (`gen/tilr/<ns>/v1.ts`).

import protobuf from "protobufjs/light.js";
import { schema } from "../gen/schema.ts";

// Bytes are plain Uint8Arrays everywhere, never Node's Buffer (which protobufjs
// picks when it finds one, as under Deno's npm compatibility).
protobuf.util.Buffer = null as unknown as typeof protobuf.util.Buffer;
protobuf.configure();

/** The protobufjs root of every bus schema (`crates/tilr-bus/proto/`). */
export const root: protobuf.Root = protobuf.Root.fromJSON(
  schema as unknown as protobuf.INamespace,
);

/** How decoded messages look: numbers for 64-bit integers and enums, defaults set. */
const DECODE: protobuf.IConversionOptions = {
  longs: Number,
  enums: Number,
  defaults: true,
  oneofs: true,
};

/** Encodes and decodes one message type of the bus schemas. */
export interface MessageCodec<T> {
  /** The full protobuf name, `tilr.pane.v1.CreateRequest`. */
  readonly name: string;
  encode(message: T): Uint8Array;
  /** A payload that does not decode throws. */
  decode(bytes: Uint8Array): T;
}

/** The codec of the message `name` (`tilr.pane.v1.CreateRequest`). */
export function codec<T>(name: string): MessageCodec<T> {
  let type: protobuf.Type | undefined;
  const lookup = () => (type ??= root.lookupType(name));
  return {
    name,
    encode(message: T): Uint8Array {
      const t = lookup();
      return t.encode(t.fromObject(message as Record<string, unknown>)).finish();
    },
    decode(bytes: Uint8Array): T {
      const t = lookup();
      return t.toObject(t.decode(bytes), DECODE) as T;
    },
  };
}
