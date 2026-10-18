# Proposal: tilr checks who serves its socket; the token goes to loopback only

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found (findings #4, #10, #20): `tilr` never checks the peer uid of the local socket (`crates/tilr/src/conn.rs:314-332`, `daemon.rs:73-77`), so another user who pre-creates `/tmp/tilr-1000` and serves a fake `tilrd` captures keystrokes; the stored token is sent to any `--connect` / `TILR_CONNECT` URL (`target.rs:99-103,116-124`); the lock and log files are opened following symlinks and the lock is truncated (`local.rs:90-93,159-166`, `daemon.rs:136-170`).

## What Changes

- `tilr` checks, before it talks, that the runtime directory is owned by the user, is 0700 and is not a symlink, and that the socket's peer uid equals its own euid; otherwise it refuses with a clear error.
- The token from the state directory is sent only to loopback targets (`127.0.0.1`, `::1`, `localhost`); a remote target needs the token given explicitly.
- Lock and log files are opened with `O_NOFOLLOW`, and the lock is not truncated before it is held.

## Capabilities

### Modified Capabilities
- `tilr-cli`: see `specs/tilr-cli/spec.md`.
- `daemon-lifecycle`: see `specs/daemon-lifecycle/spec.md`.

## Out of scope

- Server-side peer checks (already in `access-control`).

## Impact

`crates/tilr/src/{conn,daemon,target}.rs`, `crates/tilr-proto/src/local.rs`. **BREAKING** for users who relied on the stored token reaching a remote `--connect` target: migration is `TILR_TOKEN` or `client.toml`'s token for that target. Nothing crosses a reload; no protocol change.
