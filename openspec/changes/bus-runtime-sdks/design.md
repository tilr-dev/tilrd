# Design: Deno and Python client libraries for the bus

## Context

Prototype (`runtime-tmp/runtimes/deno/bus.ts`, `runtimes/python/bus.py`): `new Bus()` /
`Bus()`, `register(method, handler)` before `connect()`, `call(method, bytes, {target, traceId})`,
`ctx.from`, `ctx.traceId`, `ctx.method`, `ctx.call` passing the trace, `BusError(code, detail)`,
`bus.closed` / `run_forever()`; Deno uses `node:net` `Socket({fd})` and `npm:protobufjs@7.4.0`
loading `../../proto/bus.proto` at run time; Python uses asyncio and a generated `bus_pb2.py`;
`AttachIdent` (Python) detects a taken-over attach token, not needed without attach instances.
The repo is a Rust workspace with strict CI (fmt, clippy, nextest, deny, gitleaks, MSRV).

## Goals / Non-goals

Goals: the prototype's surface, made safe (backpressure, limits, cancellation) and tested in CI
against the real broker. Non-goals: see proposal.

## Decisions

### D1. Where they live
`sdk/deno/` and `sdk/python/` at the repo root, next to `crates/`: they are not Cargo crates and
must not enter the workspace's build. Rejected: inside `crates/tilr-bus/` (cargo would package
them; mixes toolchains in one crate directory).

### D2. API surface (both, idiomatic names)
| | Deno | Python |
|---|---|---|
| create | `new Bus()` | `Bus()` |
| register (before connect) | `bus.register(m, async (payload, ctx) => bytes \| {payload, contentType})` | `bus.register(m, handler)` / `@bus.method(m)` |
| connect | `await bus.connect()` (env) | `await bus.connect()` |
| call | `await bus.call(m, bytes, {target, timeoutMs, contentType, credential})` → `Uint8Array` | `await bus.call(m, payload, target=, timeout_ms=, content_type=, credential=)` → `bytes` |
| full reply | `callReply` → `{payload, contentType}` | `call_reply` → `Reply` |
| context | `ctx.from`, `ctx.method`, `ctx.traceId`, `ctx.deadline`, `ctx.signal`, `ctx.credential`, `ctx.call` | `ctx.from_`, `ctx.method`, `ctx.trace_id`, `ctx.deadline`, `ctx.cancelled`, `ctx.credential`, `ctx.call` |
| identity | `bus.instance` (from `HelloAck`) | `bus.instance` |
| own token | `bus.credential` (latest `CredentialUpdate`) | `bus.credential` |
| end | `await bus.closed` | `await bus.wait_closed()` |
| JSON helpers | `json.encode/decode` | `tilr_bus.json.dumps/loads` |

`ctx.call` passes the trace id and the time left (`timeout_ms`), not the credential unless asked
(`credential: ctx.credential`), as `bus-acl-biscuit` D4 says.

### D3. Errors
`BusError(code, message)`; constants for the `component-bus` codes. A handler that throws
`BusError` replies with its code; any other exception replies `handler_error` with the exception's
type and message (no traceback to the caller; the traceback goes to stderr, hence the server
log). An incoming call for a method not registered replies `not_supported`. Local errors:
`too_large` (before sending), `unavailable` (connection closed), `timeout` is the broker's.

### D4. Backpressure and limits
Python: every write is followed by `await writer.drain()`; Deno: a write that returns `false`
waits for `drain`. At most `HelloAck.max_in_flight` own calls pending: further `call`s wait for a
slot (not an error; the broker's `busy` remains for callee queues). Frames over `max_frame`:
`too_large` locally. Incoming calls run as separate tasks (as in the prototype); a handler that
blocks the event loop synchronously stops the whole runtime (documented).

### D5. Cancellation
On `Cancel{id}` the context's signal is aborted (Deno) or the handler's task is cancelled
(Python, `asyncio.CancelledError` inside the handler); the library sends no reply for it.

### D6. Schema and generated code
Single source `crates/tilr-bus/proto/bus.proto`. `scripts/gen-bus-sdk.sh` writes
`sdk/python/src/tilr_bus/bus_pb2.py` (`protoc --python_out`, the pinned protoc) and
`sdk/deno/proto.ts` (the schema as a string constant for `protobufjs.parse`, so the module needs
no file access); both checked in; `--check` regenerates into a temp directory and diffs. Pinned
runtime dependencies: Deno `npm:protobufjs@7.4.0` (in `deno.json` imports and `deno.lock`),
Python `protobuf` (exact version in `pyproject.toml`, `uv.lock` committed).

### D7. Tests against the real broker
Each library has a test runtime (`sdk/deno/test/runtime.ts`, `sdk/python/tests/runtime.py`) and a
test suite (`deno test -A`, `uv run pytest`) that: builds or finds `target/debug/tilrd`
(`TILR_TEST_TILRD`), starts it with a temporary `TILR_CONFIG_DIR`, `TILR_STATE_DIR`,
`TILR_RUNTIME_DIR`, `--no-listen`, and a `bus.toml` whose instances run the test runtime of the
library under test (both transports), and calls through `tilr bus call` or a second instance.
Cross-language case: a Python instance calls a Deno instance and back.

### D8. CI
New job `sdk` (needs `lint` like `test`): `scripts/install-tools.sh DIR protoc deno uv`, `cargo
build -p tilrd -p tilr --locked`, `scripts/gen-bus-sdk.sh --check`, `deno fmt --check`,
`deno lint`, `deno test -A` in `sdk/deno`, `uv run ruff check`, `uv run pytest` in `sdk/python`.
Pull requests: Linux; merge queue, release branches and nightly: Linux and macOS (the two-tier
rule of AGENTS.md). Deno and uv pinned with SHA-256 sums in `install-tools.sh` like the other
tools. Locally the job is `scripts/sdk-check.sh`, named in AGENTS.md's check list.

## Reload and reconnect

A tilrd reload keeps the instances and their connections (`component-bus` D13): the libraries
see calls in flight answered `unavailable` and nothing else. A lost connection rejects pending
calls with `unavailable` and resolves `closed`; the runner restarts the process, so the library
does not reconnect.

## Backpressure

D4.

## Risks / Trade-offs

- Two more toolchains in CI; pinned and hash-checked like Zig and protoc.
- `protobufjs` is an npm dependency of a Deno module; a hand-written codec would remove it but
  adds code to keep in step with the schema.

## Open Questions

- Publishing (JSR `@tilr/bus`, PyPI `tilr-bus`) and the versioning of the SDKs relative to
  tilrd. Rec.: not now; path/git dependencies until the protocol is public, then publish with the
  release that makes it public.
- Minimum versions: Deno 2.x, Python 3.12. Rec.: these (the prototype runs on them).
