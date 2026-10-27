# Proposal: Adopt the tilr-dev repository standard

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The repository moved to GitHub `tilr-dev/tilrd` (folder `core` to `tilrd`, branch `cloud` to `main`, history kept) and took the organization standard copied from gitpm (Basic Memory `main/projects/tilr/tilrd-repozitorii-servera-na-git-hub-tilr-dev-tilrd-standart-org-ci-relizy-cdn`).

## What Changes

- Dual MIT/Apache-2.0 license files; workspace metadata (`rust-version = "1.90"`, repository, per-crate description and keywords); `deny.toml`; `AGENTS.md`; English README, TODO and USE_CASES.
- CI: fmt and clippy, tests on Linux and macOS, MSRV, cargo-deny, gitleaks.
- Release workflow: release-please (`simple`) tags `vX.Y.Z`, builds macOS and Linux binaries, attaches them to the GitHub release and uploads them to the CDN through `scripts/release.sh --upload-only`; Linux aarch64 gated by `RELEASE_LINUX_ARM64`.
- Zig and protoc installed by `scripts/install-tools.sh` with pinned checksums; `release.sh` requires `CLOUDFLARE_ACCOUNT_ID`; rustls 0.23.46 for RUSTSEC-2026-0346..0349.

## Capabilities

### Modified Capabilities
- `release-process`

## Out of scope

- Porting CI and release to the `tilrd`/`tilr` names of the itests branch (in progress separately).
- A scoped R2 token for the `cdn` environment (open change `scoped-cdn-token`).

## Impact

Implemented by: [`0696a88`](https://github.com/tilr-dev/tilrd/commit/0696a88) chore: adopt the tilr-dev repo standard (2026-10-16).
