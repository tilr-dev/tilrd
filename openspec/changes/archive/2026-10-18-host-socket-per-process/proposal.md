# Proposal: Two servers of one user keep their own host-events sockets

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #11): the host-events socket path is keyed by `server_id`, so two `tilrd` of one user (two Unix sockets, one state directory, hence one `server_id`) share the path and unlink each other's socket. Re-checked at 1163abd: the path is `<server_id>.sock` in the runtime directory (`host.rs:88-90`); `host::bind` removes any socket file already there, live or not (`host.rs:115-131`); a stopping server removes the path unconditionally (`supervisor.rs:592-594`, `host.rs:134-137`). The second server takes the first one's path, and whichever stops first removes the other's socket: `tilr edit` / `tilr open` in the panes of the survivor then fail.

## Decision (2026-10-18, technical)

- The first server keeps `<server_id>.sock`; a server that finds a live server answering there uses `<server_id>-<pid>.sock` instead. The pid survives a reload in place, so the successor finds its own path again; panes created before this change keep a working `TILR_SOCK` (the common single-server case does not change path), so no compatibility path is needed.
- Rejected: always `<server_id>-<pid>.sock` (every pane started before the update would lose `tilr edit` until recreated, for a case that needs two servers); handing the listening socket across the exec in the state file (a state format change for a rare case).

## What Changes

- At start (and after a reload) the server picks its host socket path: `<server_id>-<pid>.sock` if a socket of that name exists (its own, from before the exec); otherwise `<server_id>.sock` unless a connection to it is accepted (another live server); otherwise `<server_id>-<pid>.sock`.
- The server removes its host socket on stop only if the file at that path is still the socket it bound (same device and inode).
- At start, `<server_id>-<pid>.sock` files whose pid is not running are removed.
- `TILR_SOCK` of each pane names its own server's socket, as now.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`: see `specs/clipboard-host-events/spec.md`.

## Non-goals

- Giving two servers of one user different server ids.
- Closing the window during a reload in which a second server starting at that moment takes `<server_id>.sock` (the reloading server then moves to `<server_id>-<pid>.sock`, and its older panes reach the other server; both servers must be started within the same milliseconds).

## Impact

`crates/tilrd/src/host.rs` (path choice, probe, removal by inode, sweep), `crates/tilrd/src/supervisor.rs` (`set_up_host_socket`, removal on stop). Survives a reload: same pid, so the same path is chosen. An update from the previous release keeps `<server_id>.sock` for the single-server case, so running panes are unaffected. No protocol change, no state file change.

Implemented by: [`ddcc453`](https://github.com/tilr-dev/tilrd/commit/ddcc453) fix(host): give each of one user's servers its own host socket (2026-10-18).
