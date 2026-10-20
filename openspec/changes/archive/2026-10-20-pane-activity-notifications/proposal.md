# Proposal: PaneActivity: tell clients that a pane rang the bell or finished a command

## Why

A lightweight event from a pane nobody watches was requested on 2026-10-03; what counts as an "event" was open: output after silence, the foreground process finishing, a bell, a title change (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`). A related idea is events, wake-up hooks and timers for agents (`main/projects/tilr/tilr-ideia-sobytiia-mcp-khuki-probuzhdeniia-agentov-i-taimery`).

## Decision (2026-10-18)

- Two events: a bell (BEL in the pane's output) and a foreground command finished (the pty's foreground process group returns to the shell's).
- Not now: output after silence (later, as a per-pane opt-in); title change.

Nothing of it exists at 1163abd: no `PaneActivity` in `crates/tilr-proto/proto/tilr.proto`; `crates/tilr-screen/src/ghostty.rs:62-81` registers `on_pty_write`, `on_title_changed` and `on_pwd_changed` but not `on_bell` (which libghostty-vt 0.2.2 offers, `Terminal::on_bell`); the pane process (`crates/tilrd/src/pane.rs:346-400`) never reads the pty's foreground process group.

## What Changes

- New server push `PaneActivity { pane_id, kind, at_ms }`, `kind` = `BELL` or `COMMAND_FINISHED`, sent to every greeted client whether or not it watches the pane; the client decides whether to notify.
- The pane process detects both: the screen model's bell, and the pty's foreground process group (`tcgetpgrp`) going from another group back to the shell's, checked after each output read and once a second.
- At most one `BELL` per pane per second; `COMMAND_FINISHED` is not limited (one per command).
- Per the protocol rule of `protocol-v3` (design D1), a push a client may ignore needs no version bump and no feature (it cannot be switched off).
- The pane process tells the supervisor over a new link packet kind (design D4); a supervisor that does not know it ignores it.

## Capabilities

### Modified Capabilities
- `panes`: see `specs/panes/spec.md`.

## Non-goals

- Output after silence and title changes as events.
- Push notifications to devices, unread markers in `PaneInfo` (client side).
- Agent hooks and timers (a later change on top of this one).

## Impact

`crates/tilr-proto/proto/tilr.proto` (one `ServerMessage` variant and its enum, additive), `crates/tilr-screen/src/{lib,ghostty}.rs` (bell callback, `Screen::take_bells`), `crates/tilrd/src/pane.rs` (detection), `crates/tilrd/src/link.rs` (packet kind), `crates/tilrd/src/supervisor.rs` (fan-out), `crates/tilrd/tests/{panes,reload}.rs`, README "Protocol", USE_CASES.md. Not breaking: no PROTOCOL_VERSION bump, no feature. Survives a reload without new state: after the exec the pane reads the foreground group again and knows whether a command runs (it is not the shell's group), so no pane-state format bump; a bell's rate-limit timestamp is not kept. Clients reconnect after a reload and miss events sent while they were away (no replay).

Implemented by: [`04c0e39`](https://github.com/tilr-dev/tilrd/commit/04c0e39) Сообщать клиентам о bell и завершении команды в панели (2026-10-20).
