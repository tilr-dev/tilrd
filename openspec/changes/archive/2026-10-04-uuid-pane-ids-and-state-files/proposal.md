# Proposal: UUID pane ids, tilr-server ps, a server id, state handed over by file

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A pane id was a counter that restarted at 1 with every server, so a remembered id could name another pane after a restart and ids from two machines clashed; UUID v7 was rejected for its shared prefix (commit message; Basic Memory `main/projects/tilr/tilr-core-id-panelei-uuid-tilr-server-ps-i-peredacha-sostoianiia-pri-perezagruzke`).

## What Changes

- Pane ids are UUID v4 in the protocol, in the process list (`pane --id <uuid>`), in the shell (`$TILR_PANE`) and in the new `tilr-server ps` (PANE, PID, SIZE, VIEWERS, CWD, COMMAND).
- `HelloAck.server_id`, a UUID kept in `server-id` in the state directory.
- A reloading process execs with its original arguments and leaves its state in a file found by name (`server-<pid>.state`, `panes/<uuid>.state`, 0600 in a 0700 directory); leftovers older than an hour are swept; `--cols`/`--rows` only when not 80x24.
- **BREAKING**: protocol version 2 (v1 clients refused) and state formats 2; a server from an earlier release (up to `0.1.0-af46f7d`) does not reload into this one and has to be restarted. No migration (testing phase).

## Capabilities

### Modified Capabilities
- `panes`
- `reload`
- `protocol`

## Out of scope

- Migration of v1 clients or format-1 state.

## Impact

Implemented by: [`865d647`](https://github.com/tilr-dev/tilrd/commit/865d647) Panes are named by UUIDs; a reload leaves the command line as it was (2026-10-04).
