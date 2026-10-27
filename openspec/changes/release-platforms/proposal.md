# Proposal: More release platforms: musl, macOS on the CDN, x86_64 builds

Зависит от: public-repo-readiness

## Why

Questions from 2026-10-04 still open: releases need glibc 2.36+ (built on bookworm), musl not tried; macOS releases and installer; the x86_64 build under emulation on an arm Mac fails (Zig 0.15.2 panic in `Random.shuffle`), so cross-compile or use an x86 builder. The pb server needed a self-built x86_64 binary because the CDN had only linux-aarch64 (Basic Memory `main/projects/tilr/tilr-ustanovka-i-obnovlenie-install.sh-relizy-na-r2-tilr-update`, `main/projects/tilr/tilr-server-na-pb.rexuni.dev-systemd-cloudflare-tunnel-tilr-pb.rexuni.dev`).

## What Changes

- Add `linux-x86_64-musl` / `linux-aarch64-musl` (static) if Zig and libghostty build there; decide whether macOS binaries go to the CDN for non-app installs.

## Capabilities

### Modified Capabilities
- `release-process`
- `installation`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- The Mac app's own feed.

## Impact

`scripts/release.sh`, release workflow, CDN layout (new platform names are additive; `latest` per platform). Installer platform detection.
