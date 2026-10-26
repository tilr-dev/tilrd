# Proposal: Green Linux CI; dependency updates

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The first CI runs on Linux failed: a clippy lint on `openpty`'s window size, a hanging test client, races in the files and reconnect tests and ETXTBSY in the update tests; Dependabot opened PRs #2, #3 and #4 (commit messages; Basic Memory `main/projects/tilr/tilrd-repozitorii-servera-na-git-hub-tilr-dev-tilrd-standart-org-ci-relizy-cdn`).

## What Changes

- `openpty` takes the window size as `&raw mut`, clean on Linux and macOS.
- The test client waits for one deadline in all (server pings no longer restart it); the files, edit, update and login-shell tests no longer race or fail on Debian's profile.
- `scripts/linux.sh` runs as a non-root `runner` user with a passwd entry and a home (`scripts/linux-entrypoint.sh`), `TILR_LINUX_USER=root` for the old behaviour.
- The reconnect test waits for the server to drop the viewer before counting (#6).
- Dependency bumps: upload/download-artifact actions; zstd 0.14, getrandom 0.4, sha2 0.11, tokio 1.53.3, tokio-tungstenite 0.30, toml 1.1, uuid 1.28, serde_json; the pty is registered with `AsyncFd::register` because tokio deprecated `AsyncFd::new`.

## Capabilities

### Modified Capabilities
- `release-process`

## Out of scope

- Known flakes still open (see the open change `test-coverage-gaps`).

## Impact

Implemented by: [`417a7b9`](https://github.com/tilr-dev/tilrd/commit/417a7b9) fix: pass the Linux CI checks (#5) (2026-10-17); [`f12d73d`](https://github.com/tilr-dev/tilrd/commit/f12d73d) test: wait for the server to drop the viewer before counting (#6) (2026-10-17); [`89fe7e6`](https://github.com/tilr-dev/tilrd/commit/89fe7e6) chore(deps): bump actions/upload-artifact from 7.0.1 to 7.0.2 (#2) (2026-10-17); [`ea541b8`](https://github.com/tilr-dev/tilrd/commit/ea541b8) chore(deps): bump actions/download-artifact from 8.0.1 to 8.0.2 (#3) (2026-10-17); [`c556192`](https://github.com/tilr-dev/tilrd/commit/c556192) chore(deps): bump the cargo group across 1 directory with 8 updates (#4) (2026-10-17). CI green with 6 checks after PR #5. The release PR #1 (0.1.1) was not merged, so the build, upload and cdn jobs have not run for real.
