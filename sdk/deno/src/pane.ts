// Typed helpers for the core's pane methods and events (`tilr.pane.v1`, design T2):
// a script creates a pane, types into it and reads its screen without encoding
// protobuf by hand.

import type { DescMessage, MessageInitShape, MessageShape } from "@bufbuild/protobuf";
import type { Bus, BusEvent, CallOptions, StreamOptions, Subscription } from "./bus.ts";
import * as pane from "../gen/tilr/pane/v1.ts";
import type { MessageCodec } from "./schema.ts";

/** The window clients open `pane.output` with (frames with images). */
export const PANE_OUTPUT_WINDOW = 4 << 20;

/** The events of a pane and their payloads. */
const EVENTS = {
  created: pane.PaneInfo,
  changed: pane.PaneInfo,
  bell: pane.PaneActivity,
  command_finished: pane.PaneActivity,
  copied: pane.Copied,
  exited: pane.PaneExited,
} as const;

export type PaneEventName = keyof typeof EVENTS;
type EventPayload<E extends PaneEventName> = (typeof EVENTS)[E] extends MessageCodec<infer D>
  ? MessageShape<D>
  : never;

/** What `pane.input` sends: text typed as is, a named key, a paste, or raw bytes. */
export type PaneInput =
  | { text: string }
  | { key: pane.NamedKey; mods?: number }
  | { paste: string }
  | { raw: Uint8Array };

/** `pane.output` as decoded frames; `end` is the pane's `PaneExited` once it ended. */
export class PaneOutput implements AsyncIterableIterator<pane.PaneOutput> {
  end?: pane.PaneExited;

  constructor(readonly chunks: ReturnType<Bus["stream"]>) {}

  [Symbol.asyncIterator](): this {
    return this;
  }

  async next(): Promise<IteratorResult<pane.PaneOutput>> {
    const next = await this.chunks.next();
    if (next.done) {
      if (this.chunks.end && this.chunks.end.length > 0) {
        this.end = pane.PaneExited.decode(this.chunks.end);
      }
      return { done: true, value: undefined };
    }
    return { done: false, value: pane.PaneOutput.decode(next.value) };
  }

  return(): Promise<IteratorResult<pane.PaneOutput>> {
    this.chunks.cancel();
    return Promise.resolve({ done: true, value: undefined });
  }

  cancel(): void {
    this.chunks.cancel();
  }
}

/** The core's pane methods on one connection (`bus.pane`). */
export class Panes {
  constructor(readonly bus: Bus) {}

  async #call<Req extends DescMessage, Resp extends DescMessage>(
    method: string,
    request: MessageCodec<Req>,
    response: MessageCodec<Resp>,
    message: MessageInitShape<Req>,
    options?: CallOptions,
  ): Promise<MessageShape<Resp>> {
    const reply = await this.bus.call(method, request.encode(message), options);
    return response.decode(reply.payload);
  }

  /** Create a pane (of a profile); answered with its `PaneInfo` once its program runs. */
  create(request: pane.CreateRequestInit = {}, options?: CallOptions): Promise<pane.PaneInfo> {
    return this.#call("pane.create", pane.CreateRequest, pane.PaneInfo, request, options);
  }

  list(options?: CallOptions): Promise<pane.PaneInfo[]> {
    return this.#call("pane.list", pane.ListRequest, pane.ListReply, {}, options).then((reply) =>
      reply.panes
    );
  }

  /** Close a pane; answered with its exit code once its program ended. */
  close(paneId: string, options?: CallOptions): Promise<number> {
    return this.#call("pane.close", pane.CloseRequest, pane.CloseReply, { paneId }, options).then((
      reply,
    ) => reply.exitCode);
  }

  /** Send input; answered once it was applied. */
  input(paneId: string, input: PaneInput, options?: CallOptions): Promise<pane.InputAck> {
    let request: pane.InputRequestInit;
    if ("text" in input) request = { paneId, input: { case: "key", value: { text: input.text } } };
    else if ("key" in input) {
      request = {
        paneId,
        input: { case: "key", value: { named: input.key, mods: input.mods ?? 0 } },
      };
    } else if ("paste" in input) {
      request = { paneId, input: { case: "paste", value: { text: input.paste } } };
    } else request = { paneId, input: { case: "raw", value: { data: input.raw } } };
    return this.#call("pane.input", pane.InputRequest, pane.InputAck, request, options);
  }

  /** Type `text` and press Enter, as two inputs applied in order. */
  async typeLine(paneId: string, text: string, options?: CallOptions): Promise<void> {
    await this.input(paneId, { raw: new TextEncoder().encode(text) }, options);
    await this.input(paneId, { key: pane.NamedKey.ENTER }, options);
  }

  resize(request: pane.ResizeRequestInit, options?: CallOptions): Promise<pane.InputAck> {
    return this.#call("pane.resize", pane.ResizeRequest, pane.InputAck, request, options);
  }

  /** The screen as text; with `sinceSeq` it waits for a change after it. */
  read(request: pane.ReadRequestInit, options?: CallOptions): Promise<pane.ReadReply> {
    return this.#call("pane.read", pane.ReadRequest, pane.ReadReply, request, options);
  }

  /**
   * Read the screen until a line satisfies `match` (each read waits for a change);
   * resolves with that line, or fails when the program exits first or the deadline
   * (`timeoutMs`, default 10 s) passes.
   */
  async waitForLine(
    paneId: string,
    match: (line: string) => boolean,
    timeoutMs = 10_000,
  ): Promise<string> {
    const deadline = Date.now() + timeoutMs;
    let sinceSeq: bigint | undefined;
    for (;;) {
      const left = deadline - Date.now();
      if (left <= 0) throw new Error(`no matching line in pane ${paneId} within ${timeoutMs} ms`);
      const screen = await this.read({
        paneId,
        sinceSeq,
        waitMs: sinceSeq === undefined ? undefined : left,
      }, {
        timeoutMs: left + 1000,
      });
      const found = screen.lines.find(match);
      if (found !== undefined) return found;
      if (!screen.alive) throw new Error(`pane ${paneId} exited before a matching line`);
      sinceSeq = screen.seq;
    }
  }

  profiles(options?: CallOptions): Promise<pane.ProfileInfo[]> {
    return this.#call("pane.profiles", pane.ProfilesRequest, pane.ProfilesReply, {}, options).then((
      reply,
    ) => reply.profiles);
  }

  /** The pane's screen as a stream of `PaneOutput` (frames, images). */
  output(
    paneId: string,
    request: { cellWidthPx?: number; cellHeightPx?: number } = {},
    options: StreamOptions = {},
  ): PaneOutput {
    const payload = pane.OutputRequest.encode({ paneId, ...request });
    return new PaneOutput(
      this.bus.stream("pane.output", payload, { window: PANE_OUTPUT_WINDOW, ...options }),
    );
  }

  /** Subscribe to `pane.<event>` with its payload decoded. */
  on<E extends PaneEventName>(
    event: E,
    handler: (payload: EventPayload<E>, event: BusEvent) => void | Promise<void>,
  ): Promise<Subscription> {
    const codec = EVENTS[event] as unknown as { decode(bytes: Uint8Array): EventPayload<E> };
    return this.bus.on(
      `pane.${event}`,
      (received) => handler(codec.decode(received.payload), received),
    );
  }
}
