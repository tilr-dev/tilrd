# Proposal: tilr edit --wait, and a clear error for a file outside the shared directories

Зависит от: protocol-v3

## Why

Open items from the files work: `tilr edit --wait`; a clear error for paths outside the roots such as `/tmp`; the delete confirmation counts only direct entries; image and PDF preview; upload and download of large files; a SOCKS allowed-hosts setting (Basic Memory `main/projects/tilr/tilr-files-failovyi-protokol-ctx.open-i-tilr-edit-dizain-reshenie`, `main/projects/tilr/tilr-todo-failovyi-i-socks-protokoly-servera-vkliuchaiutsia-i-otkliuchaiutsia-v-konfige`).

Triage, checked at 1163abd:
- `tilr edit --wait`: not there. `tilr edit` sends one JSON line on the host socket and exits on the reply (`crates/tilrd/src/host.rs:244-305`, `crates/tilr/src/host.rs:40-80`); `HostEvent` has no id and no client message answers it (`tilr.proto`, `HostEvent`). Kept (server and protocol).
- A clear error outside the roots: not there. The host socket delivers `tilr edit /tmp/x` to the clients whatever the path (`host.rs:137-200`); the client then gets `/tmp/x: outside the directories this server shares` from `FsRead` only after it tried. Kept (server).
- Delete confirmation counting only direct entries, image and PDF preview: client UI; dropped here (Tilr app).
- Large files: downloads are already possible in parts (`FsRead` `offset`/`length` within `fs_max_bytes`, spec `files` "A read is bounded by the limit and fails whole"); uploads over `fs_max_bytes` have no recorded use. Dropped here.
- SOCKS allowed hosts: no use or design recorded (Rationale not recorded). Dropped here.

## What Changes

- `tilr edit --wait FILE` (and `tilr open --wait`) returns only when a client that opened the file says the user is done with it (closed the tab), so it works as `EDITOR` / `GIT_EDITOR`; exit 0, or 1 with the client's error.
- `HostEvent` gets `event_id` (a random 64-bit id); new `ClientMessage` `HostEventDone { event_id, error }`; a client that can send it lists `"host-events-wait"` in `Hello.features`.
- Host socket: a request with `"wait": true` is sent only when at least one receiving client lists `"host-events-wait"` (else `{"ok":false,"error":"no attached client can wait for the file"}`); the reply `{"ok":true,"clients":N,"event_id":"<id>"}` is followed by `{"done":true}` or `{"done":false,"error":"..."}` when the first such client answers. A waiting `tilr` whose connection is lost (a reload) reconnects and sends `{"name":"wait","event_id":"<id>"}` to wait on.
- With files on, a `tilr edit` (kind `file`) whose path is outside `[files] roots` is refused on the host socket with `<path>: outside the directories this server shares` and sent to no client.
- No `HelloAck.features` entry and no PROTOCOL_VERSION bump (protocol-v3 rule): the new field and message belong to `"host-events"`; a client that does not list `"host-events-wait"` is never waited on.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`: see `specs/clipboard-host-events/spec.md` (requirements added).

## Non-goals

- Client UI: delete confirmation counts, previews.
- Uploads larger than `fs_max_bytes`, a SOCKS allow-list (no recorded use; a new change when one comes).
- A timeout for `--wait` (like `code --wait`, it waits until the file is closed or the user interrupts it).

## Impact

`crates/tilr-proto/proto/tilr.proto` (`HostEvent.event_id`, `HostEventDone`; additive), `crates/tilrd/src/{host,supervisor,files}.rs` (wait registry, roots check), `crates/tilr/src/{host,main}.rs` (`--wait`), README "Files and `tilr edit`" and "Protocol", USE_CASES.md, itests UC-076 (new cases). Reload: no state crosses it; a waiting `tilr` re-registers its id with the new supervisor (design). Update from the previous release: works; a `tilr edit --wait` from the old shim does not exist.

Not breaking: additive messages and fields only, no PROTOCOL_VERSION bump, no close code, state file or CDN change.
