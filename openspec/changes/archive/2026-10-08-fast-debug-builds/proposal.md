# Proposal: Debug builds build Ghostty in ReleaseFast

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

In Debug mode Ghostty verified page integrity on every scrolled line (about 1.5 ms each): a full-screen `ls -la ~` in a 42-row pane of a debug server took 930 ms (commit message; Basic Memory `main/projects/tilr/tilr-core-debug-sborka-tormozila-na-prokrutke-ghostty-debug-fiks-i-zamer`).

## What Changes

- `[profile.dev.package.libghostty-vt-sys] debug = false`, so Zig builds that crate ReleaseFast; the same `ls` takes 59 ms. The server itself is built as before.

## Capabilities

### Modified Capabilities
- `release-process`

## Out of scope

- Nothing recorded.

## Impact

Implemented by: [`c854014`](https://github.com/tilr-dev/tilrd/commit/c854014) Dev builds: libghostty-vt-sys without debuginfo, so zig builds it ReleaseFast (2026-10-08). Measured with `tilr-test measure` in the test client.
