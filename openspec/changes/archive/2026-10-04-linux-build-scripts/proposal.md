# Proposal: README, Linux build scripts and formatting

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Rationale not recorded; the commit message says the tests and a release build were to run in a Linux container with Zig and protoc, and that `.cargo/config.toml` is a local macOS workaround.

## What Changes

- `scripts/linux.sh` runs the tests in a container with Zig and protoc; `scripts/linux-build.sh` puts a release binary into `dist/`.
- `.cargo/config.toml` is no longer tracked.
- README and TODO updated; `cargo fmt` over the workspace.

## Capabilities

### Modified Capabilities
- `release-process`

## Out of scope

- CI (added on 2026-10-16).

## Impact

Implemented by: [`b0fb083`](https://github.com/tilr-dev/tilrd/commit/b0fb083) README, Linux build scripts, TODO update (2026-10-04); [`c2fa5c4`](https://github.com/tilr-dev/tilrd/commit/c2fa5c4) cargo fmt (2026-10-04).
