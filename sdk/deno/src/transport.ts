// The transports to the broker (design B3, B10, K1): an inherited descriptor or an
// instance socket (stream framing, `node:net` under Deno), a WebSocket (TCP, or the
// server's Unix socket under Deno). In a browser only the WebSocket exists; `node:net`
// is imported only when a stream transport is used.

import {
  CodecError,
  decodeWs,
  encodeStream,
  encodeWs,
  type Envelope,
  MAX_FRAME,
  StreamDecoder,
} from "./codec.ts";

export const SUBPROTOCOL = "tilr.bus.v1";

/** How the connection ended. */
export interface CloseInfo {
  /** The WebSocket close code (4000-4003, 1002, ...); undefined on stream sockets. */
  code?: number;
  reason: string;
  /** The `ConnectionError` the broker sent, or what broke the connection. */
  error?: { code: string; message: string };
}

export type Incoming =
  | { kind: "envelope"; envelope: Envelope }
  | { kind: "bad"; message: string }
  | { kind: "closed"; info: CloseInfo };

/** One open connection to the broker, framing included. */
export interface Link {
  readonly framing: "stream" | "ws";
  /** Encode an envelope as this link frames it; over `maxFrame` throws (too large). */
  encode(envelope: Envelope, maxFrame: number): Uint8Array;
  /** Write a frame; resolves when the socket takes more (drain, `bufferedAmount`). */
  write(frame: Uint8Array): Promise<void>;
  /** The next thing that arrives (the handshake reads `HelloAck` with it). */
  take(): Promise<Incoming>;
  /** Everything that arrives after, in order; buffered until a receiver is set. */
  onIncoming(receive: (incoming: Incoming) => void): void;
  close(): void;
}

/** Hands what arrives to one receiver, keeping what came before it was set. */
class Inbox {
  #queue: Incoming[] = [];
  #receive?: (incoming: Incoming) => void;
  #taker?: (incoming: Incoming) => void;
  #closed = false;

  deliver(incoming: Incoming): void {
    if (this.#closed) return;
    if (incoming.kind === "closed") this.#closed = true;
    const taker = this.#taker;
    if (taker) {
      this.#taker = undefined;
      taker(incoming);
    } else if (this.#receive) {
      this.#receive(incoming);
    } else {
      this.#queue.push(incoming);
    }
  }

  take(): Promise<Incoming> {
    const first = this.#queue.shift();
    if (first) return Promise.resolve(first);
    return new Promise((resolve) => (this.#taker = resolve));
  }

  set(receive: (incoming: Incoming) => void): void {
    this.#receive = receive;
    for (const incoming of this.#queue.splice(0)) receive(incoming);
  }
}

/** Where to connect. */
export type Target =
  | { kind: "fd"; fd: number }
  | { kind: "socket"; path: string }
  | { kind: "ws"; url: string; origin?: string }
  | { kind: "unix-ws"; path: string; origin?: string };

export class TransportError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
    this.name = "TransportError";
  }
}

export function open(target: Target): Promise<Link> {
  switch (target.kind) {
    case "fd":
    case "socket":
      return openStream(target);
    case "ws":
    case "unix-ws":
      return openWs(target);
  }
}

// ---------------------------------------------------------------------------
// Stream sockets
// ---------------------------------------------------------------------------

interface NodeSocket {
  write(data: Uint8Array): boolean;
  on(event: string, listener: (...args: unknown[]) => void): void;
  once(event: string, listener: (...args: unknown[]) => void): void;
  off(event: string, listener: (...args: unknown[]) => void): void;
  end(): void;
  destroy(): void;
  destroyed: boolean;
}

async function openStream(
  target: { kind: "fd"; fd: number } | { kind: "socket"; path: string },
): Promise<Link> {
  const net = (await import("node:net")).default;
  const where = target.kind === "fd" ? `descriptor ${target.fd}` : target.path;
  const socket = await new Promise<NodeSocket>((resolve, reject) => {
    try {
      if (target.kind === "fd") {
        resolve(
          new net.Socket({
            fd: target.fd,
            readable: true,
            writable: true,
          }) as unknown as NodeSocket,
        );
        return;
      }
      const s = net.connect({ path: target.path }) as unknown as NodeSocket;
      s.once("connect", () => resolve(s));
      s.once("error", (err) => reject(err));
    } catch (err) {
      reject(err);
    }
  }).catch((err: Error) => {
    throw new TransportError("unavailable", `connect to ${where}: ${err.message}`);
  });
  return new StreamLink(socket);
}

class StreamLink implements Link {
  readonly framing = "stream" as const;
  #inbox = new Inbox();
  #decoder = new StreamDecoder();
  #ended = false;
  /** The wait for `drain` that every backpressured write shares. */
  #drained?: Promise<void>;

  constructor(readonly socket: NodeSocket) {
    socket.on("data", (data) => {
      // A plain view: what decodes from it is plain too (no Node Buffer).
      const bytes = data as Uint8Array;
      this.#decoder.push(new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength));
      try {
        for (let envelope = this.#decoder.next(); envelope; envelope = this.#decoder.next()) {
          this.#inbox.deliver({ kind: "envelope", envelope });
        }
      } catch (err) {
        // A bad length or frame: the stream's boundaries are lost.
        this.#end({
          reason: "",
          error: { code: "bad_frame", message: `bad packet: ${(err as Error).message}` },
        });
        socket.destroy();
      }
    });
    socket.on("error", (err) => {
      this.#end({
        reason: "",
        error: { code: "unavailable", message: `socket: ${(err as Error).message}` },
      });
    });
    socket.on("close", () => this.#end({ reason: "" }));
    socket.on("end", () => this.#end({ reason: "" }));
  }

  #end(info: CloseInfo): void {
    if (this.#ended) return;
    this.#ended = true;
    this.#inbox.deliver({ kind: "closed", info });
  }

  encode(envelope: Envelope, maxFrame: number): Uint8Array {
    return encodeStream(envelope, maxFrame);
  }

  write(frame: Uint8Array): Promise<void> {
    if (this.socket.destroyed) return Promise.reject(new Error("the socket is closed"));
    if (this.socket.write(frame)) return Promise.resolve();
    // One pair of listeners for all writes waiting, removed when the wait is over.
    return this.#drained ??= new Promise<void>((resolve, reject) => {
      const done = () => {
        this.socket.off("drain", onDrain);
        this.socket.off("close", onClose);
        this.#drained = undefined;
      };
      const onDrain = () => {
        done();
        resolve();
      };
      const onClose = () => {
        done();
        reject(new Error("the socket is closed"));
      };
      this.socket.once("drain", onDrain);
      this.socket.once("close", onClose);
    });
  }

  take(): Promise<Incoming> {
    return this.#inbox.take();
  }

  onIncoming(receive: (incoming: Incoming) => void): void {
    this.#inbox.set(receive);
  }

  close(): void {
    this.socket.end();
  }
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

/** `bufferedAmount` above which writes wait. */
const WS_HIGH_WATER = 1 << 20;

function openWs(
  target: { kind: "ws"; url: string; origin?: string } | {
    kind: "unix-ws";
    path: string;
    origin?: string;
  },
): Promise<Link> {
  const where = target.kind === "ws" ? target.url : target.path;
  // deno-lint-ignore no-explicit-any
  const deno = (globalThis as any).Deno;
  let ws: WebSocket;
  try {
    if (target.kind === "unix-ws" || target.origin) {
      if (!deno) {
        throw new Error(
          "a Unix socket or an Origin header needs Deno; a browser connects to a ws:// URL",
        );
      }
      const options: Record<string, unknown> = { protocols: [SUBPROTOCOL] };
      if (target.origin) options.headers = { origin: target.origin };
      if (target.kind === "unix-ws") {
        options.client = deno.createHttpClient({ proxy: { transport: "unix", path: target.path } });
      }
      const url = target.kind === "unix-ws" ? "ws://localhost/" : target.url;
      // deno-lint-ignore no-explicit-any
      ws = new (WebSocket as any)(url, options);
    } else {
      ws = new WebSocket(target.url, [SUBPROTOCOL]);
    }
  } catch (err) {
    return Promise.reject(new TransportError("unavailable", `${where}: ${(err as Error).message}`));
  }
  ws.binaryType = "arraybuffer";
  return new Promise((resolve, reject) => {
    let opened = false;
    const link = new WsLink(ws);
    ws.addEventListener("open", () => {
      opened = true;
      if (ws.protocol !== SUBPROTOCOL) {
        ws.close(1002, "");
        reject(new TransportError("unavailable", `${where} does not speak ${SUBPROTOCOL}`));
        return;
      }
      resolve(link);
    });
    ws.addEventListener("error", (event) => {
      if (opened) return;
      const message = (event as ErrorEvent).message || "the WebSocket handshake failed";
      const denied = / 40[13]\b/.test(message);
      reject(new TransportError(denied ? "denied" : "unavailable", `${where}: ${message}`));
    });
    ws.addEventListener("close", (event) => {
      if (opened) return;
      reject(
        new TransportError(
          "unavailable",
          `${where}: closed before it opened (${event.code} ${event.reason})`,
        ),
      );
    });
  });
}

class WsLink implements Link {
  readonly framing = "ws" as const;
  #inbox = new Inbox();

  constructor(readonly ws: WebSocket) {
    ws.addEventListener("message", (event) => {
      const data = event.data;
      if (typeof data === "string") {
        this.#inbox.deliver({ kind: "bad", message: "bad packet: a text message" });
        return;
      }
      try {
        const envelope = decodeWs(new Uint8Array(data as ArrayBuffer));
        this.#inbox.deliver({ kind: "envelope", envelope });
      } catch (err) {
        const message = err instanceof CodecError ? err.message : String(err);
        this.#inbox.deliver({ kind: "bad", message: `bad packet: ${message}` });
      }
    });
    ws.addEventListener("close", (event) => {
      // 1005 and 1006 (and Deno's 0): no close frame came.
      const code = [0, 1005, 1006].includes(event.code) ? undefined : event.code;
      this.#inbox.deliver({ kind: "closed", info: { code, reason: event.reason } });
    });
  }

  encode(envelope: Envelope, maxFrame: number): Uint8Array {
    return encodeWs(envelope, maxFrame);
  }

  async write(frame: Uint8Array): Promise<void> {
    if (this.ws.readyState !== WebSocket.OPEN) throw new Error("the WebSocket is closed");
    this.ws.send(frame);
    // A WebSocket has no drain event: wait for the buffer to go down (browser
    // `bufferedAmount`, K3).
    while (this.ws.bufferedAmount > WS_HIGH_WATER && this.ws.readyState === WebSocket.OPEN) {
      await new Promise((resolve) => setTimeout(resolve, 2));
    }
  }

  take(): Promise<Incoming> {
    return this.#inbox.take();
  }

  onIncoming(receive: (incoming: Incoming) => void): void {
    this.#inbox.set(receive);
  }

  close(): void {
    try {
      this.ws.close(1000, "");
    } catch {
      // Closed already.
    }
  }
}

export { MAX_FRAME };
