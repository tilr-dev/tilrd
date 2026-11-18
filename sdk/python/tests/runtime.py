"""The test runtime of the Python library: a bus participant for its tests (as
`sdk/deno/test/runtime.ts` is for the Deno side, with the same methods). Not part of
the library.

It connects as the environment says (`TILR_BUS_FD`; `TILR_BUS_SOCK` with
`TILR_BUS_TOKEN`; `TILR_BUS_URL`, `unix:<path>` for the server's Unix socket, with
`TILR_BUS_CREDENTIAL`; `TILR_BUS_ACCESS_TOKEN` for the owner on the TCP port), prints
`ready <identity>` on stdout once greeted, and provides, payloads as text, in the
namespace `TEST_NAMESPACE` (default `test`):

| method | answers |
|---|---|
| `echo` | the payload and content type unchanged |
| `whoami` | its identity; the owner's connections answer their client name |
| `from` | the caller's identity |
| `sleep` | `slept` after `<ms>` (default 10000); a cancellation is recorded |
| `nested` | `<wait_ms> <method> [<target>]`: waits, then calls `method` with `ctx.call` |
| `deadline` | the `timeout_ms` it was called with |
| `trace` | the trace id it was called with |
| `record` | records the payload, answers how many are recorded |
| `records` | every record, one per line, once there are at least `<n>` (default 0) |
| `count` | stream: chunks `1` .. `<n>`, then `End`; a cancel is recorded |
| `flood` | stream: `<bytes>` (default 64 MiB) in chunks of the window (64 KiB at most) |
| `flood_state` | `<state> <sent> <credited> <ahead> <window>` of the last flood |

`flood` waits for credit and records how far it got ahead of the reader's credit
(`ahead`, the most of `sent - credited` seen); `flood_state` answers once that flood
waits for credit (`waiting`), ended (`ended`) or got more than its window ahead
(`ahead`).
| `fail` | the error `<code> <message>` (default `bad_request x`) |
| `crash` | raises a plain `RuntimeError`: `handler_error` |
| `emit` | emits `<subject> <payload>` |
| `subscribe` | subscribes to `<pattern>`; records each event: `event <subject> <from> <payload>` |

`TEST_CLIENT_NAME` (else `TILR_INSTANCE`, else `python-runtime`) is the client name. It
exits 0 when its connection ends.
"""

from __future__ import annotations

import asyncio
import os
import sys
from dataclasses import dataclass

from tilr_bus import Bus, BusError, Context, StreamWriter

NS = os.environ.get("TEST_NAMESPACE") or "test"
CLIENT_NAME = (
    os.environ.get("TEST_CLIENT_NAME") or os.environ.get("TILR_INSTANCE") or "python-runtime"
)

records: list[str] = []
changed = asyncio.Condition()


@dataclass
class Flood:
    """The last flood: what it sent, what its reader credited, how far ahead it got."""

    state: str = "none"
    sent: int = 0
    credited: int = 0
    ahead: int = 0
    window: int = 0


last_flood = Flood()


async def record(line: str) -> int:
    async with changed:
        records.append(line)
        changed.notify_all()
    return len(records)


def text(payload: bytes) -> str:
    return payload.decode().strip()


def number(payload: bytes, fallback: int, method: str) -> int:
    value = text(payload)
    if not value:
        return fallback
    try:
        return int(value)
    except ValueError:
        raise BusError("bad_request", f"{method}: {value!r} is not a number") from None


bus = Bus(client_name=CLIENT_NAME, access_token=os.environ.get("TILR_BUS_ACCESS_TOKEN"))


def m(name: str) -> str:
    return f"{NS}.{name}"


@bus.provide(m("echo"))
def echo(payload: bytes, ctx: Context):
    from tilr_bus import Reply

    return Reply(payload, ctx.content_type)


@bus.provide(m("whoami"))
def whoami(payload: bytes, ctx: Context) -> str:
    return CLIENT_NAME if bus.identity == "owner" else bus.identity


@bus.provide(m("from"))
def from_(payload: bytes, ctx: Context) -> str:
    return ctx.from_


@bus.provide(m("sleep"))
async def sleep(payload: bytes, ctx: Context) -> str:
    ms = number(payload, 10_000, ctx.method)
    try:
        await asyncio.sleep(ms / 1000)
    except asyncio.CancelledError:
        await record(f"aborted {ctx.method} {ctx.id}")
        raise
    return "slept"


@bus.provide(m("nested"))
async def nested(payload: bytes, ctx: Context):
    parts = text(payload).split()
    if len(parts) < 2:
        raise BusError("bad_request", f"{ctx.method}: <wait_ms> <method> [<target>]")
    wait, method, *target = parts
    if int(wait) > 0:
        await asyncio.sleep(int(wait) / 1000)
    reply = await ctx.call(method, target=target[0] if target else None)
    return reply


@bus.provide(m("deadline"))
def deadline(payload: bytes, ctx: Context) -> str:
    return str(ctx.timeout_ms)


@bus.provide(m("trace"))
def trace(payload: bytes, ctx: Context) -> str:
    return ctx.trace_id


@bus.provide(m("record"))
async def record_method(payload: bytes, ctx: Context) -> str:
    return str(await record(payload.decode()))


@bus.provide(m("records"))
async def records_method(payload: bytes, ctx: Context) -> str:
    at_least = number(payload, 0, ctx.method)
    async with changed:
        await changed.wait_for(lambda: len(records) >= at_least)
    return "\n".join(records)


@bus.provide(m("count"))
async def count(payload: bytes, ctx: Context, out: StreamWriter) -> None:
    n = number(payload, 0, ctx.method)
    try:
        for i in range(1, n + 1):
            await out.write(str(i))
    except asyncio.CancelledError:
        await record(f"cancelled {ctx.method} {ctx.id}")
        raise


@bus.provide(m("flood"), kind="stream")
async def flood(payload: bytes, ctx: Context, out: StreamWriter) -> None:
    global last_flood
    total = number(payload, 64 << 20, ctx.method)
    size = max(1, min(out.window, 64 << 10))
    block = b"x" * size
    state = last_flood = Flood(state="sending", window=out.window)

    async def note(now: str | None = None) -> None:
        async with changed:
            state.credited = out.credited
            state.ahead = max(state.ahead, state.sent - state.credited)
            if now is not None:
                state.state = now
            elif state.ahead > state.window:
                state.state = "ahead"
            changed.notify_all()

    try:
        while state.sent < total:
            length = min(size, total - state.sent)
            # About to wait for the reader's credit: what `flood_state` waits for.
            await note("waiting" if out.would_wait(length) else None)
            await out.write(block if length == size else block[:length])
            state.sent += length
            if state.state == "waiting":
                state.state = "sending"
            await note()
        await note("ended")
    except asyncio.CancelledError:
        await note("ended")
        await record(f"cancelled {ctx.method} {ctx.id}")
        raise


@bus.provide(m("flood_state"))
async def flood_state(payload: bytes, ctx: Context) -> str:
    async with changed:
        await changed.wait_for(lambda: last_flood.state in ("waiting", "ended", "ahead"))
    state = last_flood
    return f"{state.state} {state.sent} {state.credited} {state.ahead} {state.window}"


@bus.provide(m("fail"))
def fail(payload: bytes, ctx: Context) -> None:
    value = text(payload)
    if not value:
        raise BusError("bad_request", "x")
    code, _, message = value.partition(" ")
    raise BusError(code, message)


@bus.provide(m("crash"))
def crash(payload: bytes, ctx: Context) -> None:
    raise RuntimeError("a plain failure")


@bus.provide(m("emit"))
async def emit(payload: bytes, ctx: Context) -> str:
    subject, _, rest = text(payload).partition(" ")
    await bus.emit(subject, rest)
    return "emitted"


@bus.provide(m("subscribe"))
async def subscribe(payload: bytes, ctx: Context) -> str:
    async def heard(event) -> None:
        await record(f"event {event.subject} {event.from_} {event.payload.decode()}")

    await bus.on(text(payload), heard)
    return "subscribed"


async def main() -> int:
    try:
        await bus.connect()
    except BusError as error:
        print(f"python test runtime: {error}", file=sys.stderr, flush=True)
        return 3 if error.code == "unavailable" else 4
    print(f"ready {bus.identity}", flush=True)
    info = await bus.wait_closed()
    print(f"python test runtime: connection ended: {info}", file=sys.stderr, flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
