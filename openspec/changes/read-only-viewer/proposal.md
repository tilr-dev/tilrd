# Proposal: Read-only viewer connections

Зависит от: protocol-v3

Блокер: Build read-only viewers now, and how is read-only access given? Options: (a) one long-lived viewer token per server (`tilr token --viewer`, replaced with `tilr token --viewer --new`), a viewer sees every pane; (b) per-pane share links with an expiry, minted by the owner (needs a signed-token scheme or Cloud; larger); (c) park the change until a use is named (demo, pairing, watching an agent). Recommendation: (c) until a use is named; when one is, (a) — the design below assumes (a) and is ready to build as is.

## Why

A read-only viewer was proposed on 2026-10-17: the client connects and sees a pane but cannot send anything into it; refused are Raw/Key/Paste/Resize/mouse/ClosePane/Fs*/Exec; read-only is a property of the connection (token or role), not a flag the client sets itself; a read-only viewer does not affect the pane's size; the use (a demo, watching) is to be found later. A per-device `device_id` was deferred in the same note (Basic Memory `main/projects/tilr/tilrd-idei-read-only-zritel-client-id`).

Checked at 1163abd: not there. One token per server (`<state>/token`, `crates/tilrd/src/auth.rs:17-45`), compared in `Hello` (`crates/tilrd/src/supervisor.rs:2010-2030`); every greeted connection may send everything.

## What Changes

- A second token, the viewer token, in `<state>/viewer-token` (0600), made by the first `tilr token --viewer` (`tilrd token --viewer` in the server), replaced by `tilr token --viewer --new`; a server that runs `--no-auth` has none (`tilr token --viewer` fails).
- A TCP `Hello` with the viewer token makes a read-only connection: `HelloAck.read_only` true and `HelloAck.features` empty. It may `ListPanes`, `Subscribe`, `Unsubscribe` and `Ping` and gets frames, pane lists and `PaneExited` like any client.
- Everything else from it is refused with `Error { message: "this connection is read-only" }` and has no effect: key, mouse, paste, raw input, `Scroll` (the view is the pane's, shared), `Resize`, `CreatePane`, `ClosePane`, `ClipboardPut`, `Fs*`, `Exec*`, `StreamData`/`StreamClose`. It is never sent `HostEvent`, agent streams or (with change `reverse-clipboard`) `ClipboardWrite`.
- `Viewer.read_only` in `PaneInfo.viewed_by`, so other clients see who only watches.
- Replacing the viewer token closes the read-only connections that used the old one (`Error` "the viewer token was replaced", then a normal close).
- The Unix socket and the full token are unchanged; no PROTOCOL_VERSION bump and no feature (protocol-v3 rule): `HelloAck.read_only` and `Viewer.read_only` are new fields with a harmless default; the viewer token is not something a server can be "run without" (no file, no viewers).

## Capabilities

### Modified Capabilities
- `access-control`: see `specs/access-control/spec.md` (requirements added).

## Non-goals

- `device_id` (deferred in the note).
- Per-pane or expiring viewer access (option (b) of the Блокер).
- Read-only on the Unix socket (it is the user's own).
- Hiding panes from a viewer.

## Impact

`crates/tilr-proto/proto/tilr.proto` (`HelloAck.read_only`, `Viewer.read_only`; additive), `crates/tilrd/src/{auth,supervisor,main}.rs`, `crates/tilr/src/main.rs` (`tilr token --viewer [--new]`), README "Who may connect" and "Protocol", USE_CASES.md, itests UC-03B. Reload: the viewer token is a file and is read at every `Hello`; connections reconnect after a reload and get read-only again by their token; nothing new crosses the reload. Update from the previous release: works (no file, no viewers).

Not breaking: additive messages and fields only, no PROTOCOL_VERSION bump, no close code, state file or CDN change.
