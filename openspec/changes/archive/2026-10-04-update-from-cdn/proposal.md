# Proposal: tilr-server update from the CDN; a versioned executable

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Servers on users' machines needed a way to update themselves; the earlier GitHub Releases plan was replaced by a CDN layout on Cloudflare R2 (Basic Memory `main/projects/tilr/tilr-ustanovka-i-obnovlenie-install.sh-relizy-na-r2-tilr-update`).

## What Changes

- The executable knows its version (package version, short commit, platform: `0.1.0-7b5740b (linux-aarch64)`), printed by `--version` and sent in `HelloAck.server_version`.
- CDN layout `releases/<platform>/latest` and `releases/<version>/tilr-server-<platform>` with `.sha256`, on bucket `tilr-cdn` served as `cdn.tilr.dev`.
- `update` reads `latest`, downloads, checks the checksum and that the download runs, renames it over the executable; the running server reloads itself.
- `scripts/release.sh [--publish]` builds and publishes; README documents a manual install.
- Releases `0.1.0-7b5740b`, `0.1.0-d72e2a8`, `0.1.0-af46f7d` were published.

## Capabilities

### Modified Capabilities
- `update`
- `release-process`

## Out of scope

- An installer (drafted as `sh/install.sh`, deferred: "install manually for now").
- macOS releases.

## Impact

Implemented by: [`7b5740b`](https://github.com/tilr-dev/tilrd/commit/7b5740b) tilr-server update: install the latest release from the CDN (2026-10-04); [`d72e2a8`](https://github.com/tilr-dev/tilrd/commit/d72e2a8) Document installing by hand and tilr-server update (2026-10-04).
