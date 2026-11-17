# Proposal: Deno and Python client libraries for the bus

Зависит от: component-bus

## Why

The owner decided on 2026-11-17 (session TILR-892) that the component bus of the prototype
`runtime-tmp` moves into this repo. Its runtimes are written against two small client libraries,
`runtimes/deno/bus.ts` and `runtimes/python/bus.py` (`docs/how-it-works.md` §8): one surface
(`register`, `connect`, `call`, `ctx.call`, `BusError`, transport from `TILR_BUS_FD` or
`TILR_BUS_SOCK` + `TILR_BUS_TOKEN`), single-threaded event loops where each incoming call is its
own task. Its stated limits: no backpressure (Python writes without `drain()`, Deno ignores the
result of `write`), no outbound frame limit, the `.proto` read from a relative path at run time
(Deno) and a generated `bus_pb2.py` beside the code (Python).

## What Changes

- `sdk/deno/` (module `mod.ts`, `deno.json`) and `sdk/python/` (package `tilr_bus`,
  `pyproject.toml`, run with `uv`), moved from the prototype and adapted to `tilr.bus.v1` as
  `component-bus` defines it: `timeout_ms` deadlines, `Cancel` (Deno `AbortSignal`, Python task
  cancellation), `content_type`, `HelloAck` limits, `credential` (opaque bytes; forwarding only).
- Backpressure: writes wait for the socket (`drain`), at most `max_in_flight` calls pending (more
  wait for a slot), frames over `max_frame` refused locally with `too_large`.
- Error mapping: `BusError(code, message)` with the code constants of `component-bus`;
  handler exceptions → `handler_error`; a closed connection rejects pending calls with
  `unavailable` and resolves `closed`.
- One schema: both libraries embed generated code from `crates/tilr-bus/proto/bus.proto`
  (Python `bus_pb2.py` via `protoc`; Deno a TypeScript module holding the schema for
  `protobufjs`, pinned); `scripts/gen-bus-sdk.sh --check` fails CI when they drift.
- Tests: each library has a test runtime and tests that run against a real `tilrd` (built from
  this workspace) with a temporary config directory and `bus.toml`; new CI job `sdk` on Linux
  (pull requests) and Linux + macOS (merge queue, nightly), with Deno and uv installed at pinned
  versions by `scripts/install-tools.sh`.

## Capabilities

### New Capabilities
- `bus-sdks`: what the Deno and Python libraries do, as seen by a component and by the broker.

## Non-goals

- Publishing to JSR or PyPI (they are used from the repo by path or git until the owner decides).
- Biscuit attenuation in the SDKs (bytes are forwarded; attenuation is in the Rust client).
- SDKs for other languages; a Node.js variant of the Deno module.
- Dynamic method registration (not in the protocol).

## Impact

New `sdk/deno/`, `sdk/python/`, `scripts/gen-bus-sdk.sh`, `scripts/install-tools.sh` (deno, uv),
`.github/workflows/ci.yml` (job `sdk`), `AGENTS.md` (prerequisites and the local SDK check),
README ("Components": writing a component), USE_CASES.md. No change to tilrd, tilr.proto or
reload state. `gitleaks` and the public-repo rules apply to the new directories.
