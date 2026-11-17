// Harness of the Deno library's tests: a real `tilrd` in temporary directories, the
// test runtime as a separate process, and a fake broker that speaks envelopes by hand.
//
// Every wait is bounded by the test's own deadline (`scenario`): a regression fails
// the test instead of hanging it. Nothing sleeps to wait for a condition; each wait
// is woken by what it waits for (a log line, a printed line, an envelope).

import { spawn } from "node:child_process";
import { decodeWs, encodeStream, encodeWs, type Envelope, StreamDecoder } from "../src/codec.ts";
import { SUBPROTOCOL } from "../src/transport.ts";

export const REPO = new URL("../../../", import.meta.url).pathname.replace(/\/$/, "");
export const RUNTIME = new URL("./runtime.ts", import.meta.url).pathname;

/** The tilrd to test: `TILRD`, else the workspace's debug build. */
export function tilrdPath(): string {
  const path = Deno.env.get("TILRD") || `${REPO}/target/debug/tilrd`;
  try {
    Deno.statSync(path);
  } catch {
    throw new Error(`no tilrd at ${path}: cargo build -p tilrd (or set TILRD)`);
  }
  return path;
}

/** Lines of a stream, as they come; each wait for one is woken by its arrival. */
export class Lines {
  readonly lines: string[] = [];
  #waiters: (() => void)[] = [];
  #ended = false;

  constructor(stream: ReadableStream<Uint8Array>) {
    this.#read(stream);
  }

  async #read(stream: ReadableStream<Uint8Array>): Promise<void> {
    const decoder = new TextDecoder();
    let rest = "";
    try {
      for await (const chunk of stream) {
        rest += decoder.decode(chunk, { stream: true });
        const parts = rest.split("\n");
        rest = parts.pop()!;
        for (const line of parts) this.lines.push(line);
        this.#wake();
      }
    } catch {
      // The process went away.
    }
    if (rest) this.lines.push(rest);
    this.#ended = true;
    this.#wake();
  }

  #wake(): void {
    for (const wake of this.#waiters.splice(0)) wake();
  }

  /** Wait for a line `match` takes; fails if the stream ends first. */
  async wait<T>(what: string, match: (line: string) => T | undefined): Promise<T> {
    for (let seen = 0;;) {
      for (; seen < this.lines.length; seen++) {
        const found = match(this.lines[seen]);
        if (found !== undefined) return found;
      }
      if (this.#ended) throw new Error(`${what}: the stream ended:\n${this.lines.join("\n")}`);
      await new Promise<void>((resolve) => this.#waiters.push(resolve));
    }
  }
}

/** Cleanups run when a scenario ends, last first. */
export class Scope {
  #cleanups: (() => void | Promise<void>)[] = [];

  defer(cleanup: () => void | Promise<void>): void {
    this.#cleanups.push(cleanup);
  }

  async close(): Promise<void> {
    for (const cleanup of this.#cleanups.reverse()) {
      try {
        await cleanup();
      } catch {
        // Best effort.
      }
    }
  }
}

/**
 * A test bounded by `timeoutMs` in all: when it passes, the test fails and its
 * processes are killed.
 */
export function scenario(
  name: string,
  timeoutMs: number,
  fn: (scope: Scope) => Promise<void>,
  options: { ignore?: boolean } = {},
): void {
  Deno.test({
    name,
    ignore: options.ignore,
    sanitizeOps: false,
    sanitizeResources: false,
    async fn() {
      const scope = new Scope();
      let timer: ReturnType<typeof setTimeout> | undefined;
      const deadline = new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${name}: not done within ${timeoutMs} ms`)),
          timeoutMs,
        );
      });
      try {
        const run = fn(scope);
        run.catch(() => {}); // after the deadline, what it still does is not this test's failure
        await Promise.race([run, deadline]);
      } finally {
        clearTimeout(timer);
        await scope.close();
      }
    },
  });
}

/** A running tilrd in its own temporary directories. */
export class Tilrd {
  constructor(
    readonly dir: string,
    readonly child: Deno.ChildProcess,
    readonly log: Lines,
    /** `ws://127.0.0.1:<port>` of the TCP port. */
    readonly url: string,
  ) {}

  /** The server's Unix socket. */
  get socket(): string {
    return `${this.dir}/runtime/tilrd.sock`;
  }

  /** `unix:<path>`: the owner on the Unix socket, as `TILR_BUS_URL` names it. */
  get unixUrl(): string {
    return `unix:${this.socket}`;
  }

  /** The access token of the TCP port. */
  token(): string {
    return Deno.readTextFileSync(`${this.dir}/state/token`).trim();
  }

  static async start(scope: Scope, files: Record<string, string> = {}): Promise<Tilrd> {
    // Short: the Unix socket's path must fit sun_path (104 bytes on macOS).
    const dir = Deno.makeTempDirSync({ dir: "/tmp", prefix: "tb" });
    scope.defer(() => Deno.removeSync(dir, { recursive: true }));
    for (const sub of ["config", "state", "runtime"]) {
      Deno.mkdirSync(`${dir}/${sub}`, { mode: 0o700 });
    }
    for (const [name, text] of Object.entries(files)) {
      Deno.writeTextFileSync(`${dir}/config/${name}`, text);
    }
    const child = new Deno.Command(tilrdPath(), {
      args: ["--listen", "127.0.0.1:0"],
      clearEnv: true,
      env: {
        TILR_RUNTIME_DIR: `${dir}/runtime`,
        TILR_STATE_DIR: `${dir}/state`,
        TILR_CONFIG_DIR: `${dir}/config`,
        HOME: dir,
        PATH: "/usr/bin:/bin",
        SHELL: "/bin/sh",
        PS1: "$ ",
        RUST_LOG: "info",
      },
      stdin: "null",
      stdout: "null",
      stderr: "piped",
    }).spawn();
    scope.defer(async () => {
      try {
        child.kill("SIGKILL");
      } catch {
        // Gone already.
      }
      await child.status;
    });
    const log = new Lines(child.stderr);
    const url = await log.wait(
      "tilrd did not start",
      (line) => line.split("listening on ")[1]?.trim(),
    );
    return new Tilrd(dir, child, log, url.startsWith("ws") ? url : `ws://${url}`);
  }

  /** Kill the server outright. */
  async kill(): Promise<void> {
    this.child.kill("SIGKILL");
    await this.child.status;
  }
}

/** A test runtime process (`test/runtime.ts`). */
export class Runtime {
  constructor(readonly child: Deno.ChildProcess, readonly stdout: Lines, readonly stderr: Lines) {}

  /** The identity it printed once greeted. */
  ready(): Promise<string> {
    return this.stdout.wait(
      "the runtime did not connect",
      (line) => line.startsWith("ready ") ? line.slice("ready ".length) : undefined,
    );
  }

  static spawn(scope: Scope, env: Record<string, string>): Runtime {
    const child = new Deno.Command(Deno.execPath(), {
      args: ["run", "--quiet", "-A", RUNTIME],
      clearEnv: true,
      env: {
        PATH: "/usr/bin:/bin",
        HOME: Deno.env.get("HOME") ?? "/tmp",
        ...denoCacheEnv(),
        ...env,
      },
      stdin: "null",
      stdout: "piped",
      stderr: "piped",
    }).spawn();
    scope.defer(async () => {
      try {
        child.kill("SIGKILL");
      } catch {
        // Gone already.
      }
      await child.status;
    });
    return new Runtime(child, new Lines(child.stdout), new Lines(child.stderr));
  }

  /** Start one and wait until it is greeted; returns it with its identity. */
  static async start(scope: Scope, env: Record<string, string>): Promise<Runtime> {
    const runtime = Runtime.spawn(scope, env);
    await runtime.ready();
    return runtime;
  }
}

function denoCacheEnv(): Record<string, string> {
  const out: Record<string, string> = {};
  for (const name of ["DENO_DIR", "XDG_CACHE_HOME"]) {
    const value = Deno.env.get(name);
    if (value) out[name] = value;
  }
  return out;
}

// ---------------------------------------------------------------------------
// The fake broker
// ---------------------------------------------------------------------------

/** The broker's end of one connection, driven step by step by a test. */
export class Peer {
  #queue: Envelope[] = [];
  #waiters: (() => void)[] = [];
  #closed = false;

  constructor(
    readonly sendFrame: (envelope: Envelope) => void,
    readonly closeLink: (code?: number, reason?: string) => void,
  ) {}

  /** Called with what the client sent. */
  _deliver(envelope: Envelope | undefined): void {
    if (envelope) this.#queue.push(envelope);
    else this.#closed = true;
    for (const wake of this.#waiters.splice(0)) wake();
  }

  send(envelope: Envelope): void {
    this.sendFrame(envelope);
  }

  /** The next envelope, `undefined` once the client closed the connection. */
  async tryRecv(): Promise<Envelope | undefined> {
    while (this.#queue.length === 0) {
      if (this.#closed) return undefined;
      await new Promise<void>((resolve) => this.#waiters.push(resolve));
    }
    return this.#queue.shift();
  }

  async recv(): Promise<Envelope> {
    const envelope = await this.tryRecv();
    if (!envelope) throw new Error("the client closed the connection");
    return envelope;
  }

  /** Take the client's Hello and answer `HelloAck` with `ack`'s fields. */
  async greet(ack: Record<string, unknown> = {}): Promise<Record<string, unknown>> {
    const hello = await this.recv();
    if (hello.msg !== "hello") throw new Error(`expected Hello, got ${hello.msg}`);
    this.send({ msg: "helloAck", helloAck: helloAck(ack) });
    return hello.hello;
  }

  /** Close the connection; a WebSocket with `code` and `reason` (default 1000). */
  close(code?: number, reason?: string): void {
    this.closeLink(code, reason);
  }
}

export function helloAck(fields: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    protocolVersion: 1,
    identity: "owner",
    connectionId: 7,
    maxFrame: 16 << 20,
    maxInFlight: 256,
    streamWindow: 1 << 20,
    connectionWindow: 1 << 20,
    serverId: "server-1",
    ...fields,
  };
}

interface NodeSocket {
  write(data: Uint8Array): boolean;
  on(event: string, listener: (...args: unknown[]) => void): void;
  destroy(): void;
}

function streamPeer(socket: NodeSocket): Peer {
  const decoder = new StreamDecoder();
  const peer = new Peer((envelope) => socket.write(encodeStream(envelope)), () => socket.destroy());
  socket.on("data", (data) => {
    const bytes = data as Uint8Array;
    decoder.push(new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength));
    for (let envelope = decoder.next(); envelope; envelope = decoder.next()) {
      peer._deliver(envelope);
    }
  });
  socket.on("close", () => peer._deliver(undefined));
  socket.on("error", () => peer._deliver(undefined));
  return peer;
}

/** A Unix socket the fake broker listens on; `accept` takes one connection. */
export async function listenUnix(scope: Scope, path: string): Promise<{ accept(): Promise<Peer> }> {
  const net = (await import("node:net")).default;
  const pending: NodeSocket[] = [];
  const waiters: ((socket: NodeSocket) => void)[] = [];
  const server = net.createServer((socket: unknown) => {
    const waiter = waiters.shift();
    if (waiter) waiter(socket as NodeSocket);
    else pending.push(socket as NodeSocket);
  });
  await new Promise<void>((resolve) => server.listen(path, resolve));
  scope.defer(() => new Promise<void>((resolve) => server.close(() => resolve())));
  return {
    async accept() {
      const socket = pending.shift() ??
        await new Promise<NodeSocket>((resolve) => waiters.push(resolve));
      scope.defer(() => socket.destroy());
      return streamPeer(socket);
    },
  };
}

/** Start the test runtime with its descriptor 3 a stream socket the fake broker holds. */
export function runtimeOnFd(
  scope: Scope,
  env: Record<string, string>,
): { peer: Peer; stdout: Lines } {
  const child = spawn(Deno.execPath(), ["run", "--quiet", "-A", RUNTIME], {
    env: {
      PATH: "/usr/bin:/bin",
      HOME: Deno.env.get("HOME") ?? "/tmp",
      ...denoCacheEnv(),
      TILR_BUS_FD: "3",
      ...env,
    },
    stdio: ["ignore", "pipe", "inherit", "pipe"],
  });
  scope.defer(() => {
    child.kill("SIGKILL");
  });
  const stdout = new Lines(
    new ReadableStream({
      start(controller) {
        child.stdout!.on("data", (data: Uint8Array) => controller.enqueue(new Uint8Array(data)));
        child.stdout!.on("end", () => controller.close());
      },
    }),
  );
  return { peer: streamPeer(child.stdio[3] as unknown as NodeSocket), stdout };
}

/** A WebSocket server for the fake broker on 127.0.0.1; `accept` takes one client. */
export function listenWs(
  scope: Scope,
): { url: string; accept(): Promise<{ peer: Peer; headers: Headers }> } {
  const pending: { peer: Peer; headers: Headers }[] = [];
  const waiters: ((accepted: { peer: Peer; headers: Headers }) => void)[] = [];
  const server = Deno.serve({ hostname: "127.0.0.1", port: 0, onListen() {} }, (request) => {
    const { socket, response } = Deno.upgradeWebSocket(request, { protocol: SUBPROTOCOL });
    socket.binaryType = "arraybuffer";
    const peer = new Peer(
      (envelope) => socket.send(encodeWs(envelope)),
      (code = 1000, reason = "") => socket.close(code, reason),
    );
    socket.addEventListener(
      "message",
      (event) => peer._deliver(decodeWs(new Uint8Array(event.data as ArrayBuffer))),
    );
    socket.addEventListener("close", () => peer._deliver(undefined));
    const accepted = { peer, headers: request.headers };
    const waiter = waiters.shift();
    if (waiter) waiter(accepted);
    else pending.push(accepted);
    return response;
  });
  scope.defer(() => server.shutdown());
  const addr = server.addr as Deno.NetAddr;
  return {
    url: `ws://127.0.0.1:${addr.port}/`,
    accept: () => {
      const accepted = pending.shift();
      return accepted ? Promise.resolve(accepted) : new Promise((resolve) => waiters.push(resolve));
    },
  };
}

export function text(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes);
}
