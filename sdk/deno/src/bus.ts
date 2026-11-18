// The bus client for Deno and browser pages (design B4-B7, K2-K3; the Rust client
// `crates/tilr-bus/src/client.rs` is the reference).
//
// A connection has one reader, which dispatches what arrives, and one writer, which
// writes what the participant sends in the order sent. Calls wait for a slot while
// `HelloAck.max_in_flight` of them are pending; sends wait while 4 MiB are queued for
// the socket. Each incoming call runs as its own task, so a handler awaiting a nested
// call holds up nobody; a `Cancel` aborts its `ctx.signal` and no reply is sent.
//
// Messages are protobuf-es's (`gen/`): 64-bit integers are `bigint` (call ids,
// `Context.id`, `connectionId`, `InputRequest.seq`, ping nonces).

import { chunkBytes, CodecError, type Envelope, MAX_FRAME } from "./codec.ts";
import { matches, validMethod, validPattern, validSubject } from "./names.ts";
import {
  type CloseInfo,
  type Incoming,
  type Link,
  open,
  type Target,
  TransportError,
} from "./transport.ts";
import { type HelloAck, RenewReply, RenewRequest } from "../gen/tilr/bus/v1.ts";
import { Panes } from "./pane.ts";

export const PROTOCOL_VERSION = 1;
export const CONTENT_TYPE_PROTOBUF = "application/x-protobuf";
export const CONTENT_TYPE_JSON = "application/json";

/** `Error.code` and `ConnectionError.code` values (design B5). */
export const codes = {
  DENIED: "denied",
  NOT_FOUND: "not_found",
  UNAVAILABLE: "unavailable",
  TIMEOUT: "timeout",
  BUSY: "busy",
  TOO_LARGE: "too_large",
  BAD_REQUEST: "bad_request",
  HANDLER_ERROR: "handler_error",
  BAD_REPLY: "bad_reply",
  NOT_SUPPORTED: "not_supported",
  CANCELLED: "cancelled",
  SPAWN_FAILED: "spawn_failed",
  START_FAILED: "start_failed",
  START_TIMEOUT: "start_timeout",
  BAD_CONFIG: "bad_config",
  EXPECTED_HELLO: "expected_hello",
  UNSUPPORTED_VERSION: "unsupported_version",
  RESERVED: "reserved",
  BAD_FRAME: "bad_frame",
} as const;

/** Environment variables of the transports (design B10, K2). */
export const env = {
  BUS_FD: "TILR_BUS_FD",
  BUS_SOCK: "TILR_BUS_SOCK",
  BUS_TOKEN: "TILR_BUS_TOKEN",
  BUS_URL: "TILR_BUS_URL",
  BUS_CREDENTIAL: "TILR_BUS_CREDENTIAL",
  INSTANCE: "TILR_INSTANCE",
  CONFIG_DIR: "TILR_CONFIG_DIR",
} as const;

/** `MethodSpec.kind`, `.lane`, `.select`. */
export const Kind = { UNARY: 0, STREAM: 1 } as const;
export const Lane = { INTERACTIVE: 0, BULK: 1 } as const;
export const Select = { ROUND_ROBIN: 0, LAST_ACTIVE: 1 } as const;

/** Bytes of envelopes queued for the socket before sends wait. */
const SEND_BUDGET = 4 << 20;
const HANDSHAKE_TIMEOUT_MS = 10_000;
/** How long a `ConnectionError` waits for the close that follows it (to keep its code). */
const CLOSE_AFTER_ERROR_MS = 2_000;
const DEFAULT_MAX_IN_FLIGHT = 256;
const DEFAULT_STREAM_WINDOW = 1 << 20;
const MAX_STREAM_WINDOW = 16 << 20;
/** Events held for a subscription whose handler is behind; more are dropped (B6). */
export const SUBSCRIPTION_QUEUE = 1024;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** A failure with a code of `codes` and a message meant for the user. */
export class BusError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
    this.name = "BusError";
  }

  override toString(): string {
    return `${this.code}: ${this.message}`;
  }
}

/** What a payload may be given as: bytes, or text sent as UTF-8. */
export type Payload = Uint8Array | string;

function bytesOf(payload: Payload | undefined): Uint8Array {
  if (payload === undefined) return new Uint8Array();
  return typeof payload === "string" ? encoder.encode(payload) : payload;
}

/** The answer to a call. */
export class Reply {
  constructor(readonly payload: Uint8Array, readonly contentType: string) {}

  /** The payload as UTF-8 text. */
  text(): string {
    return decoder.decode(this.payload);
  }
}

/** An event a subscription received. */
export interface BusEvent {
  subject: string;
  payload: Uint8Array;
  contentType: string;
  /** The emitter's identity, set by the broker. */
  from: string;
  /** `pane:<id>` for the core's pane events; empty otherwise. */
  scope: string;
}

/** A token of the server: bytes, or base64url text as `tilr bus token` prints it. */
export type Credential = Uint8Array | string;

export function credentialBytes(credential: Credential): Uint8Array {
  if (typeof credential !== "string") return credential;
  const text = credential.trim().replace(/=+$/, "").replace(/-/g, "+").replace(/_/g, "/");
  try {
    const binary = atob(text + "=".repeat((4 - (text.length % 4)) % 4));
    return Uint8Array.from(binary, (c) => c.charCodeAt(0));
  } catch {
    throw new BusError(codes.BAD_REQUEST, "credential is not base64url");
  }
}

/** Options of one call. */
export interface CallOptions {
  /** A participant's identity; unset routes by method. */
  target?: string;
  /** Sent as `Call.timeout_ms`; the call fails `timeout` locally when it passes. */
  timeoutMs?: number;
  /** Unset: `application/x-protobuf`. */
  contentType?: string;
  /** Authorizes this call alone (R4). */
  credential?: Credential;
  traceId?: string;
}

/** Options of one stream. */
export interface StreamOptions extends CallOptions {
  /** The initial credit in payload bytes; 0 or unset = `HelloAck.stream_window`. */
  window?: number;
}

/** What a unary handler may return: the reply's bytes, text, or bytes with a type. */
export type HandlerResult = Payload | { payload: Payload; contentType?: string } | void;

export type UnaryHandler = (
  payload: Uint8Array,
  ctx: Context,
) => HandlerResult | Promise<HandlerResult>;
/** A stream handler writes with `out`; what it returns is the `End` payload. */
export type StreamHandler = (
  payload: Uint8Array,
  ctx: Context,
  out: StreamWriter,
) => Payload | void | Promise<Payload | void>;

/**
 * What `provide` takes: a unary handler `(payload, ctx)`, or a stream handler
 * `(payload, ctx, out)` (told apart by its parameters, or by `ProvideOptions.kind`).
 */
export type Handler = (
  payload: Uint8Array,
  ctx: Context,
  out: StreamWriter,
) => HandlerResult | Promise<HandlerResult>;

/** How a method is provided. */
export interface ProvideOptions {
  /** `"stream"` for a stream handler; unset: a handler of three parameters is one. */
  kind?: "unary" | "stream";
  /** The lane of the consumer's connection its answers go on (default interactive). */
  lane?: "interactive" | "bulk";
  /** How a call without a target picks among providers (default round robin). */
  select?: "round_robin" | "last_active";
}

/** Options of a connection. Without a transport here, the environment names one. */
export interface ConnectOptions {
  /** A WebSocket URL (`ws://`, `wss://`), or `unix:<path>` for the server's Unix socket. */
  url?: string;
  /** For a WebSocket: a token of the server (bytes or base64url). */
  credential?: Credential;
  /** For a WebSocket on the TCP port: the owner's access token. */
  accessToken?: string;
  /** An inherited stream socket (an instance's descriptor 3). */
  fd?: number;
  /** An instance socket and its one-time token. */
  socket?: { path: string; token: string };
  /** The `Origin` header (Deno only; a browser sends its own). */
  origin?: string;
  clientName?: string;
  /** Names this participant for as long as it runs (a UUID); empty = none. */
  clientId?: string;
  /** How long connecting may take in all (default 10 s); past it, `timeout`. */
  handshakeTimeoutMs?: number;
  /** The environment to read; default `Deno.env` (none in a browser). */
  env?: Record<string, string | undefined>;
}

/** The transport the options or the environment name, in the order of K2. */
export function resolveTransport(
  options: ConnectOptions,
): {
  target: Target;
  auth: { accessToken: string; credential?: Uint8Array; instanceToken: string };
} {
  const credential = options.credential === undefined
    ? undefined
    : credentialBytes(options.credential);
  const accessToken = options.accessToken ?? "";
  if (options.fd !== undefined) {
    return { target: { kind: "fd", fd: options.fd }, auth: { accessToken: "", instanceToken: "" } };
  }
  if (options.socket) {
    return {
      target: { kind: "socket", path: options.socket.path },
      auth: { accessToken: "", instanceToken: options.socket.token },
    };
  }
  if (options.url) {
    return {
      target: wsTarget(options.url, options.origin),
      auth: { accessToken, credential, instanceToken: "" },
    };
  }
  const lookup = (name: string): string | undefined => {
    const value = options.env ? options.env[name] : denoEnv(name);
    return value ? value : undefined;
  };
  const fd = lookup(env.BUS_FD);
  if (fd !== undefined) {
    const n = Number(fd);
    if (!Number.isInteger(n) || n < 0) {
      throw new BusError(codes.BAD_REQUEST, `${env.BUS_FD}=${fd} is not a descriptor`);
    }
    return { target: { kind: "fd", fd: n }, auth: { accessToken: "", instanceToken: "" } };
  }
  const sock = lookup(env.BUS_SOCK);
  if (sock !== undefined) {
    const token = lookup(env.BUS_TOKEN);
    if (token === undefined) {
      throw new BusError(codes.UNAVAILABLE, `${env.BUS_SOCK} is set without ${env.BUS_TOKEN}`);
    }
    return {
      target: { kind: "socket", path: sock },
      auth: { accessToken: "", instanceToken: token },
    };
  }
  const url = lookup(env.BUS_URL);
  if (url !== undefined) {
    const text = lookup(env.BUS_CREDENTIAL);
    return {
      target: wsTarget(url, options.origin),
      auth: {
        accessToken,
        credential: credential ?? (text ? credentialBytes(text) : undefined),
        instanceToken: "",
      },
    };
  }
  throw new BusError(
    codes.UNAVAILABLE,
    `no bus to connect to: set ${env.BUS_FD}, ${env.BUS_SOCK} and ${env.BUS_TOKEN}, or ${env.BUS_URL} (and ${env.BUS_CREDENTIAL})`,
  );
}

function wsTarget(url: string, origin?: string): Target {
  if (url.startsWith("unix:")) return { kind: "unix-ws", path: url.slice("unix:".length), origin };
  return { kind: "ws", url, origin };
}

function denoEnv(name: string): string | undefined {
  // deno-lint-ignore no-explicit-any
  const deno = (globalThis as any).Deno;
  try {
    return deno?.env?.get(name);
  } catch {
    return undefined; // No --allow-env: as if unset.
  }
}

/** A counting semaphore whose waiters are served in order. */
class Slots {
  #free: number;
  #waiters: { resolve: () => void; reject: (err: Error) => void }[] = [];
  #closed?: Error;

  constructor(count: number) {
    this.#free = count;
  }

  acquire(): Promise<void> {
    if (this.#closed) return Promise.reject(this.#closed);
    if (this.#free > 0 && this.#waiters.length === 0) {
      this.#free--;
      return Promise.resolve();
    }
    return new Promise((resolve, reject) => this.#waiters.push({ resolve, reject }));
  }

  release(): void {
    const next = this.#waiters.shift();
    if (next) next.resolve();
    else this.#free++;
  }

  close(error: Error): void {
    this.#closed = error;
    for (const waiter of this.#waiters.splice(0)) waiter.reject(error);
  }
}

/** Credit a provider holds for one stream (B7). */
class CreditGate {
  #credit: number;
  #waiters: { bytes: number; resolve: () => void; reject: (err: Error) => void }[] = [];
  #closed = false;

  constructor(initial: number) {
    this.#credit = initial;
  }

  get available(): number {
    return this.#credit;
  }

  add(bytes: number): void {
    this.#credit += bytes;
    this.#wake();
  }

  close(): void {
    this.#closed = true;
    const error = new BusError(codes.CANCELLED, "the stream has ended");
    for (const waiter of this.#waiters.splice(0)) waiter.reject(error);
  }

  tryTake(bytes: number): boolean {
    if (this.#closed) throw new BusError(codes.CANCELLED, "the stream has ended");
    if (this.#waiters.length === 0 && this.#credit >= bytes) {
      this.#credit -= bytes;
      return true;
    }
    return false;
  }

  take(bytes: number): Promise<void> {
    if (this.tryTake(bytes)) return Promise.resolve();
    return new Promise((resolve, reject) => this.#waiters.push({ bytes, resolve, reject }));
  }

  #wake(): void {
    while (this.#waiters.length > 0 && this.#credit >= this.#waiters[0].bytes) {
      const waiter = this.#waiters.shift()!;
      this.#credit -= waiter.bytes;
      waiter.resolve();
    }
  }
}

/** What a handler knows of the call it answers. */
export class Context {
  /** When the caller's time is up (`Date.now()` milliseconds); unset without a deadline. */
  readonly deadline?: number;

  constructor(
    readonly bus: Bus,
    /** The call's id on this connection. */
    readonly id: bigint,
    readonly method: string,
    /** The caller's identity, set by the broker. */
    readonly from: string,
    readonly contentType: string,
    readonly traceId: string,
    /** `Call.timeout_ms` as received: the time the caller had left (0 = none). */
    readonly timeoutMs: number,
    /** The per-call credential the caller presented (empty = none). */
    readonly credential: Uint8Array,
    /** Aborted when the call is cancelled (the caller's `Cancel`, its deadline). */
    readonly signal: AbortSignal,
  ) {
    if (timeoutMs > 0) this.deadline = Date.now() + timeoutMs;
  }

  /** Milliseconds until the deadline, unset without one. */
  timeLeftMs(): number | undefined {
    return this.deadline === undefined ? undefined : Math.max(0, this.deadline - Date.now());
  }

  /** A nested call carrying this call's trace id and the time it has left. */
  call(method: string, payload?: Payload, options: CallOptions = {}): Promise<Reply> {
    return this.bus.call(method, payload, this.#nested(options));
  }

  /** A nested stream, as `call`. */
  stream(method: string, payload?: Payload, options: StreamOptions = {}): ChunkStream {
    return this.bus.stream(method, payload, this.#nested(options));
  }

  #nested<T extends CallOptions>(options: T): T {
    const left = this.timeLeftMs();
    return {
      ...options,
      traceId: options.traceId ?? (this.traceId || undefined),
      timeoutMs: options.timeoutMs ?? (left === undefined ? undefined : Math.max(1, left)),
    };
  }
}

/** Writes the chunks of a provided stream, within the credit its reader gave. */
export class StreamWriter {
  readonly #served: Served;

  constructor(
    readonly bus: Bus,
    readonly id: bigint,
    readonly gate: CreditGate,
    /** The reader's initial window; no chunk may be larger. */
    readonly window: number,
    served: Served,
  ) {
    this.#served = served;
  }

  /** Payload bytes this provider may send now. */
  get credit(): number {
    return this.gate.available;
  }

  /** Payload bytes the reader's `Credit`s gave back so far (beyond the window). */
  get credited(): number {
    return this.#served.credited;
  }

  /** Whether `write` of `bytes` bytes would wait for credit now. */
  wouldWait(bytes: number): boolean {
    return this.gate.available < bytes;
  }

  /** Send a chunk once the reader's credit allows it. */
  async write(payload: Payload, contentEncoding = ""): Promise<void> {
    const bytes = this.#check(payload);
    await this.gate.take(bytes.length);
    await this.bus._send({ msg: "chunk", chunk: { id: this.id, payload: bytes, contentEncoding } });
  }

  /** Send a chunk only if the credit allows it now; false when it does not. */
  async tryWrite(payload: Payload, contentEncoding = ""): Promise<boolean> {
    const bytes = this.#check(payload);
    if (!this.gate.tryTake(bytes.length)) return false;
    await this.bus._send({ msg: "chunk", chunk: { id: this.id, payload: bytes, contentEncoding } });
    return true;
  }

  #check(payload: Payload): Uint8Array {
    const bytes = bytesOf(payload);
    if (bytes.length > this.window) {
      throw new BusError(
        codes.TOO_LARGE,
        `a chunk of ${bytes.length} bytes is larger than the stream's window of ${this.window}`,
      );
    }
    return bytes;
  }
}

type StreamItem = { chunk: Uint8Array; carried: number } | { end: Uint8Array } | {
  error: BusError;
};

interface CallerStream {
  push(item: StreamItem): void;
  allowed: number;
  received: number;
  /** Set once the `Call` is queued for the socket: a `Cancel` may follow it. */
  queued: boolean;
  /** Cancelled before its `Call` was queued: the `Cancel` goes out after it. */
  cancelled: boolean;
  timer?: ReturnType<typeof setTimeout>;
}

/**
 * The chunks of a stream as an async iterator of payloads (decompressed when their
 * producer compressed them). The call is sent when the iteration begins (the first
 * `next()`), as in the Python library; credit goes back as each chunk is taken;
 * leaving the loop early cancels the stream. `end` holds the `End` payload once it
 * ended.
 */
export class ChunkStream implements AsyncIterableIterator<Uint8Array> {
  /** The `End` payload, once the stream ended. */
  end?: Uint8Array;
  id = 0;
  #items: StreamItem[] = [];
  #wake?: () => void;
  #done = false;
  #start?: (stream: ChunkStream) => Promise<void>;
  #started?: Promise<void>;
  #cancelled = false;

  constructor(readonly bus: Bus, start: (stream: ChunkStream) => Promise<void>) {
    this.#start = start;
  }

  /** Called by the bus with what arrives. */
  _push(item: StreamItem): void {
    this.#items.push(item);
    this.#wake?.();
  }

  get _cancelled(): boolean {
    return this.#cancelled;
  }

  [Symbol.asyncIterator](): this {
    return this;
  }

  async next(): Promise<IteratorResult<Uint8Array>> {
    if (this.#done) return { done: true, value: undefined };
    if (this.#start) {
      const start = this.#start;
      this.#start = undefined;
      this.#started = start(this).catch((error) => this._push({ error: toBusError(error) }));
    }
    await this.#started;
    while (this.#items.length === 0) {
      if (this.#done) return { done: true, value: undefined }; // cancelled meanwhile
      await new Promise<void>((resolve) => (this.#wake = resolve));
      this.#wake = undefined;
    }
    const item = this.#items.shift()!;
    if ("chunk" in item) {
      this.bus._credit(this.id, item.carried);
      return { done: false, value: item.chunk };
    }
    this.#finish();
    if ("end" in item) {
      this.end = item.end;
      return { done: true, value: undefined };
    }
    throw item.error;
  }

  /** Leaving a `for await` loop early: cancel the stream. */
  return(): Promise<IteratorResult<Uint8Array>> {
    this.cancel();
    return Promise.resolve({ done: true, value: undefined });
  }

  /** Stop the stream (`Cancel`); a stream not begun is never sent. */
  cancel(): void {
    if (this.#done) return;
    this.#cancelled = true;
    this.#start = undefined;
    this.bus._cancelStream(this.id);
    this.#finish();
    this.#wake?.();
  }

  #finish(): void {
    if (this.#done) return;
    this.#done = true;
    this.bus._releaseStream(this);
  }
}

function toBusError(error: unknown): BusError {
  if (error instanceof BusError) return error;
  if (error instanceof CodecError) {
    return new BusError(error.tooLarge ? codes.TOO_LARGE : codes.BAD_REQUEST, error.message);
  }
  if (error instanceof TransportError) return new BusError(error.code, error.message);
  return new BusError(codes.UNAVAILABLE, error instanceof Error ? error.message : String(error));
}

interface PendingCall {
  resolve: (reply: Reply) => void;
  reject: (error: BusError) => void;
}

interface Served {
  abort: AbortController;
  gate?: CreditGate;
  /** Bytes the reader's `Credit`s gave (counted apart from the gate). */
  credited: number;
}

/** A subscription: callable to end it, with how many events it dropped. */
export interface Subscription {
  /** End the subscription (`Unsubscribe`). */
  (): void;
  unsubscribe(): void;
  readonly pattern: string;
  /**
   * Events dropped because `SUBSCRIPTION_QUEUE` of them already waited for the
   * handler (the newest are dropped; the connection and the subscription stay).
   */
  readonly dropped: number;
}

/** Delivers a subscription's events to its handler one at a time, in order. */
class EventQueue {
  #queue: BusEvent[] = [];
  #running = false;
  dropped = 0;

  constructor(
    readonly pattern: string,
    readonly handler: (event: BusEvent) => void | Promise<void>,
  ) {}

  deliver(event: BusEvent): void {
    if (this.#queue.length >= SUBSCRIPTION_QUEUE) {
      this.dropped++;
      return;
    }
    this.#queue.push(event);
    if (!this.#running) this.#run();
  }

  close(): void {
    this.#queue = [];
  }

  async #run(): Promise<void> {
    this.#running = true;
    try {
      for (let event = this.#queue.shift(); event; event = this.#queue.shift()) {
        try {
          await this.handler(event);
        } catch (error) {
          console.error(`tilr bus: handler of ${this.pattern}: ${error}`);
        }
      }
    } finally {
      this.#running = false;
    }
  }
}

interface Provided {
  spec: { method: string; kind: number; lane: number; select: number };
  unary?: UnaryHandler;
  stream?: StreamHandler;
}

/**
 * A connection to the bus. Provide methods with `provide` before `connect`; then
 * `call`, `stream`, `emit`, `on`.
 *
 * ```ts
 * const bus = new Bus({ clientName: "demo" });
 * bus.provide("demo.echo", (payload) => payload);
 * await bus.connect();
 * const reply = await bus.call("test.echo", "hi");
 * ```
 */
export class Bus {
  #options: ConnectOptions;
  #provided = new Map<string, Provided>();
  #link?: Link;
  #ack?: HelloAck;
  #maxFrame = MAX_FRAME;
  #streamWindow = DEFAULT_STREAM_WINDOW;
  #inFlight = new Slots(DEFAULT_MAX_IN_FLIGHT);
  #nextId = 0;
  #calls = new Map<number, PendingCall>();
  #streams = new Map<number, CallerStream>();
  #subscriptions = new Map<number, EventQueue>();
  #served = new Map<bigint, Served>();
  #queue: { frame: Uint8Array; budgeted: boolean }[] = [];
  #queued = 0;
  #budgetWaiters: (() => void)[] = [];
  #writing = false;
  #credential?: Uint8Array;
  #closeInfo?: CloseInfo;
  #closedResolve!: (info: CloseInfo) => void;
  /** Resolves when the connection has ended, saying how. */
  readonly closed: Promise<CloseInfo>;
  #connecting = false;
  #panes?: Panes;
  #pendingError?: { code: string; message: string };
  #pendingTimer?: ReturnType<typeof setTimeout>;

  constructor(options: ConnectOptions = {}) {
    this.#options = options;
    this.closed = new Promise((resolve) => (this.#closedResolve = resolve));
  }

  /** Connect with `options` (or the environment) and say Hello. */
  static connect(options: ConnectOptions = {}): Promise<Bus> {
    return new Bus(options).connect();
  }

  /**
   * Provide `method`, answered by `handler`; only before `connect` (there is no
   * later registration, B4). Name the kind: `{ kind: "stream" }` for a stream handler
   * `(payload, ctx, out)`, `{ kind: "unary" }` (the default for a handler of fewer
   * parameters) for one that answers once. Without `kind`, a handler declaring three
   * parameters is taken for a stream handler; a wrapped or variadic function may not
   * declare them, so pass `kind`.
   */
  provide(method: string, handler: Handler, options: ProvideOptions = {}): this {
    if (this.#link || this.#connecting) {
      throw new BusError(
        codes.BAD_REQUEST,
        `provide ${method} before connect: there is no later registration`,
      );
    }
    if (!validMethod(method)) {
      throw new BusError(codes.BAD_REQUEST, `${JSON.stringify(method)} is not a method name`);
    }
    const stream = options.kind ? options.kind === "stream" : handler.length >= 3;
    this.#provided.set(method, {
      spec: {
        method,
        kind: stream ? Kind.STREAM : Kind.UNARY,
        lane: options.lane === "bulk" ? Lane.BULK : Lane.INTERACTIVE,
        select: options.select === "last_active" ? Select.LAST_ACTIVE : Select.ROUND_ROBIN,
      },
      unary: stream ? undefined : handler as UnaryHandler,
      stream: stream ? handler as StreamHandler : undefined,
    });
    return this;
  }

  /** Connect, say `Hello` with every provided method, wait for `HelloAck`. */
  async connect(options: ConnectOptions = {}): Promise<this> {
    if (this.#link || this.#connecting) throw new BusError(codes.BAD_REQUEST, "connected already");
    const merged = { ...this.#options, ...options };
    // Before anything is under way: a transport that cannot be named leaves the Bus
    // as it was, to connect again.
    const { target, auth } = resolveTransport(merged);
    this.#connecting = true;
    const timeoutMs = merged.handshakeTimeoutMs ?? HANDSHAKE_TIMEOUT_MS;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let link: Link | undefined;
    let expired = false;
    const handshake = (async () => {
      const opened = await open(target);
      if (expired) {
        opened.close(); // past the deadline: no Hello, the socket is not kept
        throw new BusError(codes.TIMEOUT, "the handshake timed out");
      }
      link = opened;
      const hello = {
        protocolVersion: PROTOCOL_VERSION,
        clientName: merged.clientName ?? "",
        clientId: merged.clientId ?? "",
        accessToken: auth.accessToken,
        credential: auth.credential ?? new Uint8Array(),
        instanceToken: auth.instanceToken,
        provides: [...this.#provided.values()].map((p) => p.spec),
      };
      await link.write(link.encode({ msg: "hello", hello }, MAX_FRAME));
      const incoming = await link.take();
      if (incoming.kind === "closed") {
        const info = incoming.info;
        if (info.error) throw new BusError(info.error.code, info.error.message);
        throw new BusError(
          codes.UNAVAILABLE,
          info.code === undefined
            ? "closed before HelloAck"
            : `closed before HelloAck (${info.code} ${info.reason})`,
        );
      }
      if (incoming.kind === "bad") throw new BusError(codes.BAD_FRAME, incoming.message);
      const envelope = incoming.envelope;
      if (envelope.msg === "connectionError") {
        throw new BusError(envelope.connectionError.code, envelope.connectionError.message);
      }
      if (envelope.msg !== "helloAck") {
        throw new BusError(codes.BAD_REPLY, "the broker did not answer Hello with HelloAck");
      }
      const ack = envelope.helloAck;
      if (ack.protocolVersion !== PROTOCOL_VERSION) {
        throw new BusError(
          codes.UNSUPPORTED_VERSION,
          `unsupported protocol version ${ack.protocolVersion}: this client speaks ${PROTOCOL_VERSION}`,
        );
      }
      return ack;
    })();
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(
        () => {
          expired = true;
          reject(new BusError(codes.TIMEOUT, `no HelloAck within ${timeoutMs} ms of connecting`));
        },
        timeoutMs,
      );
    });
    let ack: HelloAck;
    try {
      ack = await Promise.race([handshake, deadline]);
    } catch (error) {
      handshake.catch(() => {});
      link?.close();
      this.#connecting = false;
      throw toBusError(error);
    } finally {
      clearTimeout(timer);
    }
    this.#ack = ack;
    this.#maxFrame = Math.min(ack.maxFrame || MAX_FRAME, MAX_FRAME);
    this.#streamWindow = ack.streamWindow || DEFAULT_STREAM_WINDOW;
    this.#inFlight = new Slots(ack.maxInFlight || DEFAULT_MAX_IN_FLIGHT);
    const minted = ack.credential;
    this.#credential = minted && minted.length > 0 ? minted : auth.credential;
    this.#link = link!;
    this.#connecting = false;
    // What arrived with HelloAck (a call, an event) is dispatched from here on.
    link!.onIncoming((incoming) => this.#receive(incoming));
    return this;
  }

  /** Typed helpers for the core's `pane.*` methods and events. */
  get pane(): Panes {
    return (this.#panes ??= new Panes(this));
  }

  /** This connection's identity, set by the broker: `owner`, `instance:<name>`, `token:<name>`. */
  get identity(): string {
    return this.#ack?.identity ?? "";
  }

  /** The broker's id of this connection (0n before `connect`). */
  get connectionId(): bigint {
    return this.#ack?.connectionId ?? 0n;
  }

  /** The `HelloAck` as received; unset before `connect`. */
  get helloAck(): Readonly<HelloAck> | undefined {
    return this.#ack;
  }

  /** The token this connection holds: the one presented, an instance's from HelloAck, or the latest `CredentialUpdate`. */
  get credential(): Uint8Array | undefined {
    return this.#credential;
  }

  get isClosed(): boolean {
    return this.#closeInfo !== undefined;
  }

  /** Present a new token on this connection (after `bus.renew`). */
  async updateCredential(credential: Credential): Promise<void> {
    const bytes = credentialBytes(credential);
    await this._send({ msg: "credentialUpdate", credentialUpdate: { credential: bytes } });
    this.#credential = bytes;
  }

  /** Renew this connection's token (`bus.renew`) and present the new one. */
  async renew(): Promise<Uint8Array> {
    if (!this.#credential) {
      throw new BusError(codes.BAD_REQUEST, "this connection holds no token to renew");
    }
    const reply = await this.call(
      "bus.renew",
      RenewRequest.encode({ credential: this.#credential }),
    );
    const renewed = RenewReply.decode(reply.payload).credential ?? new Uint8Array();
    await this.updateCredential(renewed);
    return renewed;
  }

  /**
   * Call `method` and wait for its reply. Waits for a slot while `max_in_flight` calls
   * and streams are pending; a payload over `max_frame` fails `too_large` unsent.
   */
  async call(method: string, payload?: Payload, options: CallOptions = {}): Promise<Reply> {
    this.#requireOpen();
    await this.#inFlight.acquire().catch(() => {
      throw this.#closedError();
    });
    try {
      if (this.#closeInfo) throw this.#closedError();
      const id = ++this.#nextId;
      const answer = new Promise<Reply>((resolve, reject) =>
        this.#calls.set(id, { resolve, reject })
      );
      answer.catch(() => {}); // Awaited below; failing before that is not unhandled.
      try {
        await this._send({ msg: "call", call: this.#callMessage(id, method, payload, options, 0) });
      } catch (error) {
        this.#calls.delete(id); // Never sent: nothing to cancel.
        throw error;
      }
      if (options.timeoutMs === undefined) return await answer;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const timeout = new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          if (this.#calls.delete(id)) {
            this._sendNow({ msg: "cancel", cancel: { id, reason: "timeout" } });
          }
          reject(
            new BusError(codes.TIMEOUT, `${method} did not answer within ${options.timeoutMs} ms`),
          );
        }, options.timeoutMs);
      });
      try {
        return await Promise.race([answer, timeout]);
      } finally {
        clearTimeout(timer);
        answer.catch(() => {});
      }
    } finally {
      this.#inFlight.release();
    }
  }

  /** Open a stream: a call answered with chunks (see `ChunkStream`). */
  stream(method: string, payload?: Payload, options: StreamOptions = {}): ChunkStream {
    return new ChunkStream(this, async (stream) => {
      this.#requireOpen();
      const requested = options.window ?? 0;
      if (requested > MAX_STREAM_WINDOW) {
        throw new BusError(
          codes.BAD_REQUEST,
          `a window of ${requested} is over the limit of ${MAX_STREAM_WINDOW}`,
        );
      }
      await this.#inFlight.acquire().catch(() => {
        throw this.#closedError();
      });
      if (this.#closeInfo || stream._cancelled) {
        this.#inFlight.release();
        if (stream._cancelled) return;
        throw this.#closedError();
      }
      const window = requested || this.#streamWindow;
      const id = ++this.#nextId;
      stream.id = id;
      const pending: CallerStream = {
        push: (item) => stream._push(item),
        allowed: window,
        received: 0,
        queued: false,
        cancelled: false,
      };
      this.#streams.set(id, pending);
      try {
        await this._send({
          msg: "call",
          call: this.#callMessage(id, method, payload, options, window),
        });
      } catch (error) {
        this.#streams.delete(id); // Never sent: nothing to cancel.
        this.#inFlight.release();
        stream.id = 0;
        throw error;
      }
      pending.queued = true;
      if (pending.cancelled) {
        // Cancelled while the Call waited for the send budget: the Cancel follows it.
        this._sendNow({ msg: "cancel", cancel: { id, reason: "cancelled by the caller" } });
        return;
      }
      if (options.timeoutMs !== undefined && this.#streams.has(id)) {
        pending.timer = setTimeout(() => {
          if (this.#streams.get(id) !== pending) return;
          this.#streams.delete(id);
          this._sendNow({ msg: "cancel", cancel: { id, reason: "timeout" } });
          pending.push({
            error: new BusError(
              codes.TIMEOUT,
              `${method} did not end within ${options.timeoutMs} ms`,
            ),
          });
        }, options.timeoutMs);
      }
    });
  }

  #callMessage(
    id: number,
    method: string,
    payload: Payload | undefined,
    options: CallOptions,
    window: number,
  ) {
    return {
      id,
      target: options.target ?? "",
      method,
      payload: bytesOf(payload),
      traceId: options.traceId ?? "",
      timeoutMs: options.timeoutMs === undefined ? 0 : Math.max(1, Math.ceil(options.timeoutMs)),
      contentType: options.contentType ?? "",
      credential: options.credential === undefined
        ? new Uint8Array()
        : credentialBytes(options.credential),
      window,
    };
  }

  /** Emit an event (at most once, to the subscribers connected now). */
  async emit(subject: string, payload?: Payload, contentType = ""): Promise<void> {
    this.#requireOpen();
    if (!validSubject(subject)) {
      throw new BusError(codes.BAD_REQUEST, `${JSON.stringify(subject)} is not an event subject`);
    }
    await this._send({ msg: "event", event: { subject, payload: bytesOf(payload), contentType } });
  }

  /**
   * Subscribe to the events matching `pattern` and call `handler` for each, in order,
   * one at a time; resolves once the broker took the subscription (`denied` if not
   * allowed) with the `Subscription`, a function that ends it. While the handler is
   * behind, up to `SUBSCRIPTION_QUEUE` (1024) events wait for it; more are dropped and
   * counted in `dropped`, as the broker drops them for a subscriber that does not read.
   */
  async on(
    pattern: string,
    handler: (event: BusEvent) => void | Promise<void>,
  ): Promise<Subscription> {
    this.#requireOpen();
    if (!validPattern(pattern)) {
      throw new BusError(
        codes.BAD_REQUEST,
        `${JSON.stringify(pattern)} is not a subscription pattern`,
      );
    }
    const id = ++this.#nextId;
    const queue = new EventQueue(pattern, handler);
    this.#subscriptions.set(id, queue);
    const answer = new Promise<Reply>((resolve, reject) =>
      this.#calls.set(id, { resolve, reject })
    );
    answer.catch(() => {}); // Awaited below; failing before that is not unhandled.
    try {
      await this._send({ msg: "subscribe", subscribe: { id, pattern } });
      await answer;
    } catch (error) {
      this.#subscriptions.delete(id);
      this.#calls.delete(id);
      throw toBusError(error);
    }
    const unsubscribe = () => {
      queue.close();
      if (this.#subscriptions.delete(id)) {
        this._sendNow({ msg: "unsubscribe", unsubscribe: { id } });
      }
    };
    return Object.defineProperties(unsubscribe, {
      unsubscribe: { value: unsubscribe },
      pattern: { value: pattern },
      dropped: { get: () => queue.dropped },
    }) as Subscription;
  }

  /** Tell the broker this participant is now the user's active one for its `LAST_ACTIVE` methods (B11). */
  async activate(): Promise<void> {
    this.#requireOpen();
    await this._send({ msg: "activate", activate: {} });
  }

  /** End the connection: pending calls fail `unavailable`, the socket is closed. */
  close(): void {
    this.#shutdown({ reason: "closed by the client" });
  }

  // -------------------------------------------------------------------------
  // Sending
  // -------------------------------------------------------------------------

  #requireOpen(): void {
    if (!this.#link) throw new BusError(codes.UNAVAILABLE, "not connected");
    if (this.#closeInfo) throw this.#closedError();
  }

  #encode(envelope: Envelope): Uint8Array {
    try {
      return this.#link!.encode(envelope, this.#maxFrame);
    } catch (error) {
      throw toBusError(error);
    }
  }

  /** Queue an envelope, waiting while the send budget is spent. @internal */
  async _send(envelope: Envelope): Promise<void> {
    if (this.#closeInfo) throw this.#closedError();
    const frame = this.#encode(envelope);
    while (this.#queued > 0 && this.#queued + frame.length > SEND_BUDGET) {
      await new Promise<void>((resolve) => this.#budgetWaiters.push(resolve));
      if (this.#closeInfo) throw this.#closedError();
    }
    this.#queued += frame.length;
    this.#queue.push({ frame, budgeted: true });
    this.#pump();
  }

  /** Queue a small control envelope at once (credits, cancels, unsubscribes). @internal */
  _sendNow(envelope: Envelope): void {
    if (this.#closeInfo || !this.#link) return;
    let frame: Uint8Array;
    try {
      frame = this.#encode(envelope);
    } catch {
      return;
    }
    this.#queue.push({ frame, budgeted: false });
    this.#pump();
  }

  async #pump(): Promise<void> {
    if (this.#writing) return;
    this.#writing = true;
    try {
      while (this.#queue.length > 0 && !this.#closeInfo) {
        const { frame, budgeted } = this.#queue.shift()!;
        try {
          await this.#link!.write(frame);
        } catch (error) {
          this.#shutdown({
            reason: "",
            error: { code: codes.UNAVAILABLE, message: `write: ${(error as Error).message}` },
          });
          return;
        }
        if (budgeted) {
          this.#queued -= frame.length;
          for (const wake of this.#budgetWaiters.splice(0)) wake();
        }
      }
    } finally {
      this.#writing = false;
    }
  }

  /** Give credit for a chunk taken. @internal */
  _credit(id: number, bytes: number): void {
    if (bytes === 0) return;
    const stream = this.#streams.get(id);
    if (!stream) return;
    stream.allowed += bytes;
    this._sendNow({ msg: "credit", credit: { id, bytes } });
  }

  /** @internal */
  _cancelStream(id: number): void {
    const pending = id ? this.#streams.get(id) : undefined;
    if (!pending) return;
    this.#forgetStream(id, pending);
    // A Call still waiting for the send budget is not queued: its Cancel follows it.
    if (pending.queued) {
      this._sendNow({ msg: "cancel", cancel: { id, reason: "cancelled by the caller" } });
    } else pending.cancelled = true;
  }

  /** @internal */
  _releaseStream(stream: ChunkStream): void {
    if (stream.id) {
      const pending = this.#streams.get(stream.id);
      if (pending) this.#forgetStream(stream.id, pending);
      this.#inFlight.release();
      stream.id = 0;
    }
  }

  /** A stream of this caller is over: no more of it is taken, its timer stops. */
  #forgetStream(id: number, pending: CallerStream): void {
    this.#streams.delete(id);
    clearTimeout(pending.timer);
  }

  #closedError(): BusError {
    const info = this.#closeInfo;
    if (info?.error) return new BusError(info.error.code, info.error.message);
    if (info?.code !== undefined) {
      return new BusError(codes.UNAVAILABLE, `connection closed (${info.code} ${info.reason})`);
    }
    return new BusError(codes.UNAVAILABLE, "connection closed");
  }

  #shutdown(info: CloseInfo): void {
    if (this.#closeInfo) return;
    this.#closeInfo = info;
    clearTimeout(this.#pendingTimer);
    const error = this.#closedError();
    for (const call of this.#calls.values()) call.reject(error);
    this.#calls.clear();
    for (const stream of this.#streams.values()) {
      clearTimeout(stream.timer);
      stream.push({ error });
    }
    this.#streams.clear();
    for (const subscription of this.#subscriptions.values()) subscription.close();
    this.#subscriptions.clear();
    for (const served of this.#served.values()) {
      served.abort.abort(error);
      served.gate?.close();
    }
    this.#served.clear();
    this.#inFlight.close(error);
    for (const wake of this.#budgetWaiters.splice(0)) wake();
    this.#queue = [];
    this.#link?.close();
    this.#closedResolve(info);
  }

  // -------------------------------------------------------------------------
  // Receiving
  // -------------------------------------------------------------------------

  #receive(incoming: Incoming): void {
    if (incoming.kind === "closed") {
      // The ConnectionError that came first says why; the close keeps its code.
      const info = incoming.info;
      this.#shutdown(
        info.error || !this.#pendingError ? info : { ...info, error: this.#pendingError },
      );
      return;
    }
    // A WebSocket keeps message boundaries: a bad one is skipped.
    if (incoming.kind === "bad") return;
    const envelope = incoming.envelope;
    switch (envelope.msg) {
      case "reply":
        this.#onReply(envelope.reply);
        break;
      case "chunk":
        this.#onChunk(envelope.chunk);
        break;
      case "end": {
        const id = Number(envelope.end.id);
        const stream = this.#streams.get(id);
        if (stream) {
          this.#forgetStream(id, stream);
          stream.push({ end: envelope.end.payload });
        }
        break;
      }
      case "event":
        this.#onEvent(envelope.event);
        break;
      case "call":
        this.#onCall(envelope.call);
        break;
      case "cancel": {
        const id: bigint = envelope.cancel.id;
        const served = this.#served.get(id);
        if (served) {
          this.#served.delete(id);
          served.abort.abort(new BusError(codes.CANCELLED, envelope.cancel.reason || "cancelled"));
          served.gate?.close();
        }
        break;
      }
      case "credit": {
        const served = this.#served.get(envelope.credit.id);
        if (served) {
          served.credited += envelope.credit.bytes;
          served.gate?.add(envelope.credit.bytes);
        }
        break;
      }
      case "credentialUpdate":
        this.#credential = envelope.credentialUpdate.credential;
        break;
      case "connectionError": {
        const error = envelope.connectionError;
        // bad_frame leaves the connection open; anything else is followed by the
        // close, whose code is kept with this error (unless it does not come).
        if (error.code !== codes.BAD_FRAME && !this.#pendingError) {
          const pending = { code: error.code, message: error.message };
          this.#pendingError = pending;
          this.#pendingTimer = setTimeout(
            () => this.#shutdown({ reason: "", error: pending }),
            CLOSE_AFTER_ERROR_MS,
          );
        }
        break;
      }
      default:
        break; // A variant of a later version, or one a participant never gets.
    }
  }

  // deno-lint-ignore no-explicit-any
  #onReply(reply: any): void {
    const id = Number(reply.id);
    const call = this.#calls.get(id);
    if (call) {
      this.#calls.delete(id);
      if (reply.result === "error") {
        call.reject(new BusError(reply.error.code, reply.error.message));
      } else call.resolve(new Reply(reply.payload ?? new Uint8Array(), reply.contentType ?? ""));
      return;
    }
    const stream = this.#streams.get(id);
    if (stream) {
      this.#forgetStream(id, stream);
      stream.push({
        error: reply.result === "error"
          ? new BusError(reply.error.code, reply.error.message)
          : new BusError(codes.BAD_REPLY, "a stream was answered with a reply instead of chunks"),
      });
    }
  }

  // deno-lint-ignore no-explicit-any
  #onChunk(chunk: any): void {
    const id = Number(chunk.id);
    const stream = this.#streams.get(id);
    if (!stream) return; // A chunk of a stream given up.
    const carried = chunk.payload.length;
    stream.received += carried;
    if (stream.received > stream.allowed) {
      this.#forgetStream(id, stream);
      stream.push({
        error: new BusError(codes.BAD_REPLY, "the provider sent more than the stream's credit"),
      });
      this._sendNow({ msg: "cancel", cancel: { id, reason: "credit overrun" } });
      return;
    }
    let bytes: Uint8Array;
    try {
      bytes = chunkBytes(chunk.payload, chunk.contentEncoding ?? "");
    } catch (error) {
      this.#forgetStream(id, stream);
      stream.push({ error: new BusError(codes.BAD_REPLY, `chunk: ${(error as Error).message}`) });
      this._sendNow({ msg: "cancel", cancel: { id, reason: "bad chunk" } });
      return;
    }
    stream.push({ chunk: bytes, carried });
  }

  // deno-lint-ignore no-explicit-any
  #onEvent(event: any): void {
    const delivered: BusEvent = {
      subject: event.subject,
      payload: event.payload,
      contentType: event.contentType ?? "",
      from: event.from ?? "",
      scope: event.scope ?? "",
    };
    for (const subscription of this.#subscriptions.values()) {
      if (matches(subscription.pattern, delivered.subject)) subscription.deliver(delivered);
    }
  }

  // deno-lint-ignore no-explicit-any
  #onCall(call: any): void {
    const id: bigint = call.id;
    const provided = this.#provided.get(call.method);
    if (!provided) {
      this._sendNow({
        msg: "reply",
        reply: {
          id,
          error: {
            code: codes.NOT_SUPPORTED,
            message: `${this.identity} does not provide ${call.method}`,
          },
        },
      });
      return;
    }
    const window = Number(call.window) || this.#streamWindow;
    const abort = new AbortController();
    const gate = provided.stream ? new CreditGate(window) : undefined;
    const served: Served = { abort, gate, credited: 0 };
    this.#served.set(id, served);
    const ctx = new Context(
      this,
      id,
      call.method,
      call.from ?? "",
      call.contentType ?? "",
      call.traceId ?? "",
      Number(call.timeoutMs ?? 0),
      call.credential ?? new Uint8Array(),
      abort.signal,
    );
    this.#serve(id, provided, call.payload ?? new Uint8Array(), ctx, served, window);
  }

  async #serve(
    id: bigint,
    provided: Provided,
    payload: Uint8Array,
    ctx: Context,
    served: Served,
    window: number,
  ): Promise<void> {
    let answer: Envelope;
    try {
      if (provided.stream) {
        const out = new StreamWriter(this, id, served.gate!, window, served);
        const end = await provided.stream(payload, ctx, out);
        answer = { msg: "end", end: { id, payload: bytesOf(end ?? undefined) } };
      } else {
        const result = await provided.unary!(payload, ctx);
        answer = { msg: "reply", reply: { id, ...replyOf(result) } };
      }
    } catch (error) {
      if (ctx.signal.aborted) return; // Cancelled: no reply.
      answer = { msg: "reply", reply: { id, error: errorOf(error) } };
    }
    // A call cancelled meanwhile gets no answer.
    if (!this.#served.delete(id)) return;
    try {
      await this._send(answer);
    } catch (error) {
      const busError = toBusError(error);
      if (busError.code === codes.TOO_LARGE) {
        await this._send({
          msg: "reply",
          reply: { id, error: { code: busError.code, message: busError.message } },
        }).catch(
          () => {},
        );
      }
    }
  }
}

function replyOf(result: HandlerResult): { payload: Uint8Array; contentType: string } {
  if (result === undefined || result === null) {
    return { payload: new Uint8Array(), contentType: "" };
  }
  if (typeof result === "string" || result instanceof Uint8Array) {
    return { payload: bytesOf(result), contentType: "" };
  }
  return { payload: bytesOf(result.payload), contentType: result.contentType ?? "" };
}

/** A handler's failure as the reply's error: a BusError keeps its code, anything else is `handler_error`. */
function errorOf(error: unknown): { code: string; message: string } {
  if (error instanceof BusError) return { code: error.code, message: error.message };
  if (error instanceof Error) {
    return { code: codes.HANDLER_ERROR, message: `${error.name}: ${error.message}` };
  }
  return { code: codes.HANDLER_ERROR, message: String(error) };
}
