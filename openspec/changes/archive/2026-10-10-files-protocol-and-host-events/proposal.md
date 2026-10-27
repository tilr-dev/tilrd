# Proposal: Files over the protocol; tilr edit / tilr open host events

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A file browser and `tilr edit` in a remote pane needed a file channel that passes through ssh-less connections; an OSC-through-the-terminal route was rejected because it does not pass ssh. Turning files and SOCKS on and off in the config was a stated must (Basic Memory `main/projects/tilr/tilr-files-failovyi-protokol-ctx.open-i-tilr-edit-dizain-reshenie`, `main/projects/tilr/tilr-todo-failovyi-i-socks-protokoly-servera-vkliuchaiutsia-i-otkliuchaiutsia-v-konfige`).

## What Changes

- Requests FsList, FsStat, FsRead, FsWrite, FsMkdir, FsRename, FsDelete, FsWatch, FsUnwatch and fs-changed events (feature `files`), rooted and size-capped by a `[files]` section (`enabled`, `roots`, `readonly`, `max_bytes`), off with `--no-files`.
- A host Unix socket the panes reach (`TILR_SOCK`), with `TILR_PANE` and `TILR_SERVER` in the pane environment and a `tilr` shim in `<runtime>/bin`.
- `tilr edit` / `tilr open` in a pane send the file or URL as a `HostEvent` to clients with the `host-events` feature.
- `tests/files.rs` (11 cases), USE_CASES.md.

## Capabilities

### Modified Capabilities
- `files`
- `clipboard-host-events`

## Out of scope

- `tilr edit --wait`, previews, large uploads (open change `files-and-edit-followups`).

## Impact

Implemented by: [`40e178e`](https://github.com/tilr-dev/tilrd/commit/40e178e) Files: the Fs* requests … and fs-changed events over the protocol …; tilr edit / tilr open … (2026-10-10). Deviations from the design noted then: `mtime_ms`, `FsEntry.target`, no transfer id in `FsData`.
