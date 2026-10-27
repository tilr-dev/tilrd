# Proposal: Repo standard and CI ported to tilrd and tilr

> Retro archive entry, reconstructed on 2026-10-18 from git history (branch `dev`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The CI, release-please and repo standard lived only on `main` and still named `tilr-server` (open item in `openspec/config.yaml` and the `release-process` spec); merging `main` into the itests work brings them over under the new names.

## What Changes

- `AGENTS.md` (and `CLAUDE.md` -> `@AGENTS.md`), LICENSE-MIT / LICENSE-APACHE, `deny.toml`, `rust-version = 1.90`.
- `.github/workflows/ci.yml` (fmt, clippy with and without `tilrd/failpoints`, tests on Linux and macOS, smoke test of `tilrd` and `tilr --version` / `--help`, MSRV, cargo-deny, gitleaks) and `release.yml` (release-please via tilr-bot, per-target builds of `tilrd` and `tilr`, `scripts/package.sh` archives `tilr-vX.Y.Z-<target>.tar.gz`, the `cdn` job with `scripts/release.sh --upload-only`, linux-aarch64 behind `RELEASE_LINUX_ARM64`).
- `release-please-config.json` for the tilrd/tilr crates, `scripts/install-tools.sh`, `scripts/package.sh`, `scripts/linux-entrypoint.sh` (non-root `runner`), `scripts/release.sh` with `--upload-only` and a required `CLOUDFLARE_ACCOUNT_ID`.
- README.md, USE_CASES.md and TODO.md in English; test harness changes (`spawn_copied` and friends in `tests/common/mod.rs`).

## Capabilities

### Modified Capabilities
- `release-process`

## Out of scope

- Behavior changes of the server or the client.
- Making the repository public (`public-repo-readiness`).

## Impact

Implemented by: [`47a8557`](https://github.com/tilr-dev/tilrd/commit/47a8557) Merge branch 'main' into dev (2026-10-18).
