# Proposal: line-tables-only debuginfo in the Linux dev container

## Why

On the host, the developer's `~/.cargo/config.toml` sets `[profile.dev] debug = "line-tables-only"`; that file is not visible inside the container `scripts/linux.sh` starts, so the container builds the dev and test profile with full debuginfo. Measured on 2026-10-21: `target/debug/tilrd` is 168 MB in the container against 37 MB on the host (about 4.5 times larger), and the `update.rs` test binary, which packages the real `tilrd` binary into release archives six times, takes about 55-65 s on Linux against about 15 s on macOS.

## What Changes

- `scripts/linux.sh` passes `CARGO_PROFILE_DEV_DEBUG=line-tables-only` into the container, so the dev profile's debuginfo there matches the host's.
- Release builds (`scripts/linux-build.sh`, `cargo build --release`) are unaffected: the variable sets only the dev profile.
- The target volume name does not change: cargo fingerprints the profile's debug setting and rebuilds the dev artifacts on its own the first time.
- Not breaking; no protocol change; nothing crosses a reload or an update.

## Capabilities

### Modified Capabilities
- `release-process`: see `specs/release-process/spec.md`.

## Non-goals

- Changing the host's cargo configuration or the tracked `.cargo/config.toml`.
- Changing the release profile or the CI runners' settings.

## Impact

`scripts/linux.sh`, `crates/tilrd/tests/release.rs`, USE_CASES.md, the `release-process` spec.
