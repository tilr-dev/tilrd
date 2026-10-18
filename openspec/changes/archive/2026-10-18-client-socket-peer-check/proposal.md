# Proposal: tilr checks who serves its socket; the token goes to loopback only

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found (findings #4, #10, #20): `tilr` never checks the peer uid of the local socket (`crates/tilr/src/conn.rs:314-332`, `daemon.rs:73-77`), so another user who pre-creates `/tmp/tilr-1000` and serves a fake `tilrd` captures keystrokes; the stored token is sent to any `--connect` / `TILR_CONNECT` URL (`target.rs:99-103,116-124`); the lock and log files are opened following symlinks and the lock is truncated (`local.rs:90-93,159-166`, `daemon.rs:136-170`).

## What Changes

- `tilr` checks, before it probes, connects or starts a server at the default socket, that the runtime directory is a directory (not a symlink) owned by its euid with no group/other permission bits (tmux's check); and for every socket (default, `TILR_SOCKET`, `--socket`, `client.toml`), after `connect()` and before the handshake, that the peer uid (`SO_PEERCRED` / `getpeereid`) equals its euid; otherwise it refuses with a clear error. A `TILR_SOCKET` in a shared directory (e.g. `/tmp/x.sock`) is not refused for its directory: the peer check covers it.
- The token from the state directory is sent only to loopback targets (`127.0.0.1`, `::1`, `localhost`); a remote target needs the token given explicitly.
- Lock and log files are opened with `O_NOFOLLOW` and must be regular files of the user's, and the lock is not truncated before it is held.

## Capabilities

### Modified Capabilities
- `tilr-cli`: see `specs/tilr-cli/spec.md`.
- `daemon-lifecycle`: see `specs/daemon-lifecycle/spec.md`.

## Out of scope

- Server-side peer checks (already in `access-control`).

## Impact

`crates/tilr/src/{conn,daemon,target,main,attach}.rs`, `crates/tilr-proto/src/local.rs`, `crates/tilrd/src/local.rs`. **BREAKING** for users who relied on the stored token reaching a remote `--connect` target: migration is `TILR_TOKEN` or `client.toml`'s token for that target. Nothing crosses a reload; no protocol change.

Implemented in: working tree on `dev` after 27be1ea, not committed yet (record the commit here when it lands).
