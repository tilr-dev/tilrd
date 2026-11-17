// The test runtime of the Deno library: a bus participant for its tests and the
// Python library's cross-language tests (as `crates/tilr-bus/examples/test_runtime.rs`
// is for the Rust side). Not part of the library.
//
// It connects as the environment says (`TILR_BUS_FD`; `TILR_BUS_SOCK` with
// `TILR_BUS_TOKEN`; `TILR_BUS_URL`, `unix:<path>` for the server's Unix socket, with
// `TILR_BUS_CREDENTIAL`; `TILR_BUS_ACCESS_TOKEN` for the owner on the TCP port), prints
// `ready <identity>` on stdout once greeted, and provides, payloads as text, in the
// namespace `TEST_NAMESPACE` (default `test`):
//
// | method | answers |
// |---|---|
// | `echo` | the payload and content type unchanged |
// | `whoami` | its identity; the owner's connections answer their client name |
// | `from` | the caller's identity |
// | `sleep` | `slept` after `<ms>` (default 10000); an abort is recorded |
// | `nested` | `<wait_ms> <method> [<target>]`: waits, then calls `method` with `ctx.call` (trace id and time left passed on) and answers its reply |
// | `deadline` | the `timeout_ms` it was called with |
// | `trace` | the trace id it was called with |
// | `record` | records the payload, answers how many are recorded |
// | `records` | every record, one per line, once there are at least `<n>` (default 0) |
// | `count` | stream: chunks `1` .. `<n>`, then `End`; a cancel is recorded |
// | `flood` | stream: `<bytes>` (default 64 MiB) in chunks of the window (at most 64 KiB), waiting for credit; records how far it got ahead of the reader's credit |
// | `flood_state` | `<state> <sent> <credited> <ahead> <window>` of the last flood once it waits for credit, ended, or got more than its window ahead (`state`: `waiting`, `ended`, `ahead`; `ahead`: the most of `sent - credited` seen) |
// | `fail` | the error `<code> <message>` (default `bad_request x`) |
// | `crash` | throws a plain `Error`: `handler_error` |
// | `emit` | emits `<subject> <payload>` |
// | `subscribe` | subscribes to `<pattern>`; each event is recorded as `event <subject> <from> <payload>` |
//
// `TEST_CLIENT_NAME` (else `TILR_INSTANCE`, else `deno-runtime`) is the client name.
// It exits 0 when its connection ends.

import { Bus, BusError, type Context, type StreamWriter } from "../mod.ts";

const ns = Deno.env.get("TEST_NAMESPACE") || "test";
const clientName = Deno.env.get("TEST_CLIENT_NAME") || Deno.env.get("TILR_INSTANCE") ||
  "deno-runtime";
const decoder = new TextDecoder();

const records: string[] = [];
const changed: (() => void)[] = [];
/** The last flood: what it sent, what its reader credited, how far ahead it got. */
const flood = { state: "none", sent: 0, credited: 0, ahead: 0, window: 0 };

function record(line: string): number {
  records.push(line);
  for (const wake of changed.splice(0)) wake();
  return records.length;
}

async function waitFor(ready: () => boolean): Promise<void> {
  while (!ready()) await new Promise<void>((resolve) => changed.push(resolve));
}

function text(payload: Uint8Array): string {
  return decoder.decode(payload).trim();
}

function number(payload: Uint8Array, fallback: number, method: string): number {
  const value = text(payload);
  if (!value) return fallback;
  const n = Number(value);
  if (!Number.isFinite(n)) {
    throw new BusError("bad_request", `${method}: ${JSON.stringify(value)} is not a number`);
  }
  return n;
}

function abortable(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => {
      clearTimeout(timer);
      reject(signal.reason);
    }, { once: true });
  });
}

const bus = new Bus({ clientName, accessToken: Deno.env.get("TILR_BUS_ACCESS_TOKEN") });
const m = (name: string) => `${ns}.${name}`;

bus.provide(m("echo"), (payload, ctx) => ({ payload, contentType: ctx.contentType }));
bus.provide(m("whoami"), () => (bus.identity === "owner" ? clientName : bus.identity));
bus.provide(m("from"), (_payload, ctx) => ctx.from);
bus.provide(m("sleep"), async (payload, ctx) => {
  const ms = number(payload, 10_000, ctx.method);
  try {
    await abortable(ms, ctx.signal);
  } catch (error) {
    record(`aborted ${ctx.method} ${ctx.id}`);
    throw error;
  }
  return "slept";
});
bus.provide(m("nested"), async (payload, ctx: Context) => {
  const [wait, method, target] = text(payload).split(/\s+/);
  if (!wait || !method) {
    throw new BusError("bad_request", `${ctx.method}: <wait_ms> <method> [<target>]`);
  }
  if (Number(wait) > 0) await abortable(Number(wait), ctx.signal);
  const reply = await ctx.call(method, undefined, target ? { target } : {});
  return { payload: reply.payload, contentType: reply.contentType };
});
bus.provide(m("deadline"), (_payload, ctx) => String(ctx.timeoutMs));
bus.provide(m("trace"), (_payload, ctx) => ctx.traceId);
bus.provide(m("record"), (payload) => String(record(decoder.decode(payload))));
bus.provide(m("records"), async (payload, ctx) => {
  const atLeast = number(payload, 0, ctx.method);
  await waitFor(() => records.length >= atLeast);
  return records.join("\n");
});
bus.provide(m("count"), async (payload: Uint8Array, ctx: Context, out: StreamWriter) => {
  const n = number(payload, 0, ctx.method);
  try {
    for (let i = 1; i <= n; i++) await out.write(String(i));
  } catch (error) {
    if (ctx.signal.aborted) record(`cancelled ${ctx.method} ${ctx.id}`);
    throw error;
  }
});
bus.provide(m("flood"), async (payload: Uint8Array, ctx: Context, out: StreamWriter) => {
  const total = number(payload, 64 << 20, ctx.method);
  const size = Math.max(1, Math.min(out.window, 64 << 10));
  const block = new Uint8Array(size).fill(0x78);
  Object.assign(flood, { state: "sending", sent: 0, credited: 0, ahead: 0, window: out.window });
  const note = (state?: string) => {
    flood.credited = out.credited;
    flood.ahead = Math.max(flood.ahead, flood.sent - flood.credited);
    if (state) flood.state = state;
    else if (flood.ahead > flood.window) flood.state = "ahead";
    for (const wake of changed.splice(0)) wake();
  };
  try {
    while (flood.sent < total) {
      const len = Math.min(size, total - flood.sent);
      // About to wait for the reader's credit: what `flood_state` waits for.
      note(out.wouldWait(len) ? "waiting" : undefined);
      await out.write(len === size ? block : block.subarray(0, len));
      flood.sent += len;
      if (flood.state === "waiting") flood.state = "sending";
      note();
    }
    note("ended");
  } catch (error) {
    note("ended");
    if (ctx.signal.aborted) record(`cancelled ${ctx.method} ${ctx.id}`);
    throw error;
  }
});
bus.provide(m("flood_state"), async () => {
  await waitFor(() => ["waiting", "ended", "ahead"].includes(flood.state));
  return `${flood.state} ${flood.sent} ${flood.credited} ${flood.ahead} ${flood.window}`;
});
bus.provide(m("fail"), (payload) => {
  const value = text(payload);
  const space = value.indexOf(" ");
  if (!value) throw new BusError("bad_request", "x");
  throw space < 0
    ? new BusError(value, "")
    : new BusError(value.slice(0, space), value.slice(space + 1));
});
bus.provide(m("crash"), () => {
  throw new Error("a plain failure");
});
bus.provide(m("emit"), async (payload) => {
  const value = text(payload);
  const space = value.indexOf(" ");
  await bus.emit(
    space < 0 ? value : value.slice(0, space),
    space < 0 ? "" : value.slice(space + 1),
  );
  return "emitted";
});
bus.provide(m("subscribe"), async (payload) => {
  await bus.on(text(payload), (event) => {
    record(`event ${event.subject} ${event.from} ${decoder.decode(event.payload)}`);
  });
  return "subscribed";
});

try {
  await bus.connect();
} catch (error) {
  console.error(`deno test runtime: ${error}`);
  Deno.exit(error instanceof BusError && error.code === "unavailable" ? 3 : 4);
}
console.log(`ready ${bus.identity}`);
const info = await bus.closed;
console.error(`deno test runtime: connection ended: ${JSON.stringify(info)}`);
Deno.exit(0);
