# Proposal: Panes on the bus

Зависит от: pane-profiles, component-bus, bus-acl-biscuit

## Why

On 2026-11-17 the owner decided (session TILR-892) that profiles must be usable by Tilr itself —
by protocol and later by runtimes on the bus — so that Tilr can launch terminals with profiles
(an agent's pane in its sandbox). The prototype's `term` runtime (`runtime-tmp/runtimes/term`,
`docs/how-it-works.md` §9) offered `term.create|list|write|read|resize|close` but kept panes only
in its own memory (lost on its reload or crash), had no terminal emulator (raw bytes in a 1 MiB
ring) and served output by long-polling `term.read` with a byte cursor, two bus calls per
keystroke. tilrd already owns panes with a screen model (`tilr-screen` on libghostty-vt), keeps
them across reloads and serves frames to clients. Panes are meant to move to a separate native
component `tilr-panes` eventually (2026-10-26); not now.

## What Changes

- tilrd itself provides methods on the bus as the reserved instance `tilrd`:
  `pane.create {profile, command?, cwd?, cols?, rows?}`, `pane.list`, `pane.write {pane_id,
  text | data_b64}`, `pane.screen {pane_id, since_seq?, wait_ms?, history_lines?}`,
  `pane.resize {pane_id, cols, rows}`, `pane.close {pane_id}` (JSON bodies).
- They map onto the existing pane supervisor: the same creation path as `CreatePane` (profiles,
  sandbox), the same input path as `RawInput`, `ClosePane`, `Resize`; panes made over the bus are
  ordinary panes: listed to every client, watchable in the Tilr app and `tilr attach`, kept across
  reloads.
- Output: `pane.screen` returns the screen model's text (rows, cursor, size, title, cwd, frame
  `seq`, exit status), optionally with history lines; with `since_seq` and `wait_ms` it waits
  (at most 25 s, below the call timeout) for the next change. No raw byte ring, no second
  emulator. A pushed stream of frames waits for the direct channel (non-goal).
- Rights (Biscuit facts, checked by tilrd as the provider with the call's token):
  `pane_profile(<name>)` / `pane_profile_any()` for `pane.create` with a profile,
  `pane_unrestricted()` for one without, `pane_override()` to override the profile's `command` or
  `cwd`, `pane_access("own" | "any")` for write/screen/resize/close/list; plus the usual
  `call("pane.…")`.
- `PaneInfo.created_by` (field 14): `owner` or `bus:<instance>`; handed over across reloads
  (`supervisor-state` 4).

## Capabilities

### New Capabilities
- `bus-panes`: pane methods on the bus, their rights, mapping onto panes.

### Modified Capabilities
- `reload`: "state-formats names what this executable reads" (`supervisor-state 4`).

## Non-goals

- Moving panes into a `tilr-panes` component.
- Streaming frames over the bus (needs the direct channel); keystroke-level key encoding
  (`KeyInput`) over the bus — `pane.write` sends bytes.
- Pane rights for tilrd's own clients (they keep full access).
- The prototype's `term.*` names and its attach client `term.py`.

## Impact

`crates/tilrd/src/bus/panes.rs` (new), `crates/tilrd/src/{supervisor,pane}.rs` (text snapshot
request to the pane process, creator), `crates/tilr-proto/proto/tilr.proto`
(`PaneInfo.created_by`; additive, no feature flag: it has a harmless default and cannot be turned
off), `crates/tilrd/src/bus/acl.rs` (pane facts), README ("Components": panes on the bus),
USE_CASES.md. Reload: `created_by` crosses it in `PaneState.info`; `supervisor-state` 3 → 4.
Not breaking on the wire.
