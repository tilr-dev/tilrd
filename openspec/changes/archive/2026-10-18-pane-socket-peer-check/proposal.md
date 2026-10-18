# Proposal: tilr edit and tilr open talk only to a host socket of the user's

## Why

Itsaplan TILR-107 (phase 1, security bug). Commit 8274039 (archived change `2026-10-18-client-socket-peer-check`) made `tilr` check, after `connect()` and before the handshake, that the peer of the server's Unix socket runs as its own euid (`check_peer`, `crates/tilr/src/conn.rs:337`, `conn.rs:349-369`), so a socket another user serves gets not a byte. Its "Out of scope" named server-side checks only (already in `access-control`). Checked at 048b0e2, the other Unix sockets:

- The host socket `$TILR_SOCK` (`tilr edit` / `tilr open` from inside a pane). Server side it is covered: the server refuses a peer of another uid (`crates/tilrd/src/host.rs:243-247`, via `local::peer`, `crates/tilrd/src/local.rs:295-306`; requirement "The host socket serves the user's own programs only"). Client side it is not: both senders connect and write the request with no peer check, the pane's `tilr` (the server's executable run through the shim, `crates/tilrd/src/host.rs:411-423`) and the `tilr` client (`crates/tilr/src/host.rs:87-98`). The request carries the pane id and the absolute path or URL to open (`host.rs:358-389`), so a socket another user serves at `$TILR_SOCK` learns which files the user opens, and its answer (`{"ok":true}`) is believed.
- Where `$TILR_SOCK` can lead elsewhere: it is inherited by every program in the pane and by what it starts (an `sudo -E`, a container, an `ssh -o SendEnv`), and a long runtime path falls back to `/tmp/tilr-<uid>/` (`crates/tilrd/src/host.rs:94-110`). The server makes and checks that directory itself (`clipboard::private_dir`, `crates/tilrd/src/clipboard.rs:54-67`), but a `tilr edit` run where the variable points at a path the user does not own has nothing but the permissions between it and another user's listener.
- The agent sockets (`agent/<pane id>.sock`): server side checked (`crates/tilrd/src/agent.rs:373-376`); the clients are the user's `ssh` / `ssh-add`, not this repo's code.
- `crates/tilr-driver` (itests only) serves no Unix socket and reaches tilrd over a TCP WebSocket only (`dial`, `crates/tilr-driver/src/session.rs:798-811`); its one Unix connection is to an SSH agent it relays for a forwarding test (`crates/tilr-driver/src/relay.rs:24-35`), a stand-in for the Tilr app's side.

## What Changes

- `tilr edit` and `tilr open`, run as the pane's `tilr` (the server's executable) or as the `tilr` client, read the peer credentials of `$TILR_SOCK` after `connect()` and before writing (`SO_PEERCRED` on Linux, `getpeereid` on macOS); a peer of another uid ends the command with exit 1 and `tilr: cannot reach the tilr server: socket <path> is served by another user (uid N), not by this one (uid M): nothing was sent`, the message `tilr` gives for its server socket since 8274039.
- One helper for "the peer of this Unix stream runs as my euid" in `tilr-proto::local` (it already has `check_runtime_dir` for both binaries), used by the two `ask` functions and by `tilr`'s `check_peer`.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`: see `specs/clipboard-host-events/spec.md` (new requirement: `tilr edit` and `tilr open` refuse a host socket another user serves).

## Non-goals

- Server-side checks: the host, agent and main sockets already refuse other uids (`access-control`).
- `crates/tilr-driver`'s SSH agent relay (`relay.rs:33`): itests-only, a stand-in for the Tilr app, and the agent it relays to is the test's own; OpenSSH's own client does not check the agent's peer either.
- The clipboard shims (`xclip`, `wl-paste`): they read files under the runtime directory, not a socket (`crates/tilrd/src/clipboard.rs:484-500`).
- Checking the directory `$TILR_SOCK` is in: the peer check covers a socket in a shared directory, as for `TILR_SOCKET` in 8274039.

## Impact

`crates/tilr-proto/src/local.rs` (the helper and its unit test), `crates/tilrd/src/host.rs` (`ask`), `crates/tilr/src/host.rs` (`ask`), `crates/tilr/src/conn.rs` (`check_peer` uses the helper), itests `UC-03A` (a new case), USE_CASES.md and itests/USE_CASES.md, README "Files and `tilr edit`" (`README.md:507-540`). No protocol change (the host socket's JSON lines are unchanged); nothing crosses a reload. An update from the previous release needs nothing: the pane's `tilr` is a symbolic link to the server's executable path (`crates/tilrd/src/clipboard.rs:91`), so it runs the installed version at once, and the `tilr` client is client-only. Not breaking for the user's own sockets.
