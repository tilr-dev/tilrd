/**
 * The tilr bus (`tilr.bus.v1`) for Deno and browser pages: call, provide, emit/on and
 * stream, over an instance's descriptor or socket or a WebSocket (design K1-K3 of
 * `openspec/design/unified-bus.md`).
 *
 * ```ts
 * import { Bus } from "./mod.ts";
 * const bus = new Bus({ clientName: "demo" });
 * bus.provide("demo.echo", (payload) => payload);
 * await bus.connect(); // TILR_BUS_FD, TILR_BUS_SOCK + TILR_BUS_TOKEN, or TILR_BUS_URL
 * console.log((await bus.call("demo.echo", "hi")).text());
 * ```
 *
 * The schemas of the core's methods are generated (`gen/`, `scripts/gen-bus-sdk.sh`):
 * `pane`, `server`, `fs`, ... each export an interface and a codec per message.
 *
 * @module
 */

export {
  Bus,
  BusError,
  type BusEvent,
  type CallOptions,
  ChunkStream,
  codes,
  type ConnectOptions,
  CONTENT_TYPE_JSON,
  CONTENT_TYPE_PROTOBUF,
  Context,
  type Credential,
  credentialBytes,
  env,
  type Handler,
  type HandlerResult,
  Kind,
  Lane,
  type Payload,
  PROTOCOL_VERSION,
  type ProvideOptions,
  Reply,
  resolveTransport,
  Select,
  type StreamHandler,
  type StreamOptions,
  StreamWriter,
  type UnaryHandler,
} from "./src/bus.ts";
export { type CloseInfo, SUBPROTOCOL } from "./src/transport.ts";
export { ENCODING_ZSTD, MAX_FRAME } from "./src/codec.ts";
export { matches, validMethod, validPattern, validSubject } from "./src/names.ts";
export {
  PANE_OUTPUT_WINDOW,
  type PaneEventName,
  type PaneInput,
  PaneOutput,
  Panes,
} from "./src/pane.ts";
export { codec, type MessageCodec } from "./src/schema.ts";

export * as agent from "./gen/tilr/agent/v1.ts";
export * as bus from "./gen/tilr/bus/v1.ts";
export * as clipboard from "./gen/tilr/clipboard/v1.ts";
export * as exec from "./gen/tilr/exec/v1.ts";
export * as fs from "./gen/tilr/fs/v1.ts";
export * as host from "./gen/tilr/host/v1.ts";
export * as pane from "./gen/tilr/pane/v1.ts";
export * as server from "./gen/tilr/server/v1.ts";
