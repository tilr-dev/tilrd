# Proposal: Deno, browser and Python libraries for the bus

Зависит от: bus-streams-events, bus-rights, pane-profiles (the typed-helper scenario creates a pane of a profile)

Стартует после: bus-wire (schemas, test runtime); its server tests need the later changes; мержится после: bus-streams-events, bus-rights, pane-profiles

Владеет: `sdk/deno/**`, `sdk/python/**`, `scripts/gen-bus-sdk.sh`, `scripts/sdk-check.sh`, the
`deno` and `uv` entries of `scripts/install-tools.sh`, the CI job `sdk` in
`.github/workflows/ci.yml`, the SDK lines of AGENTS.md (prerequisites, checks).

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` Part K,
O3): SDKs for Deno 2.x, Python 3.12 and browser JS (the Deno package over WebSocket), covering
call, provide, emit/on and stream; not published before the release that makes the protocol
public.

## What Changes

- `deno` and `uv` pinned with SHA-256 in `scripts/install-tools.sh`; `scripts/gen-bus-sdk.sh
  [--check]` generates the schema code from `crates/tilr-bus/proto/` (K4).
- `sdk/deno/` (Deno and browser) and `sdk/python/` (`tilr_bus`) with the surface of K2, the
  errors, limits and cancellation of K3, each with a test runtime and tests against a real
  `tilrd` (K5), including cross-language calls.
- CI job `sdk` (Linux on pull requests; Linux and macOS in the merge queue and nightly);
  `scripts/sdk-check.sh` locally.

## Capabilities

### New Capabilities
- `bus-sdks`: transports, bytes unchanged, error codes, concurrent handlers, events, streams
  with credit, typed helpers, deadlines and cancellation, the broker's limits, closed
  connections, the generated schema.

## Non-goals

- Publishing to JSR, PyPI or npm (shared design); changing a `.proto` (frozen, O4).

## Impact

New `sdk/`, scripts, CI, AGENTS.md; two more pinned toolchains. No server change; the tests use
instances (`bus-instances`), tokens (`bus-rights`), streams (`bus-streams-events`) and, for the
typed helpers, `pane.*` with a profile (`bus-core-panes`, `pane-profiles`). Tasks 1.1-1.3 may be
written as soon as `bus-wire` has landed. Protocol: version 1. Nothing crosses a reload; nothing released from `main` (O5).
