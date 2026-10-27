# Proposal: Writes and state files safe across a reload, a crash or a full disk

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-064 was red (the temp file of a write cut by a reload stayed for ever) and UC-065 was red (a partial state file was left on a full disk); after a reload the descriptor count of the server differed from before (itests/USE_CASES.md; Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- A write notes its temp file in `writes/` of the runtime directory; a starting or reloaded server sweeps temp files of writes whose process died or was itself.
- State files are written to a temp file next to them, synced and renamed; on error the temp file is removed.
- Inherited processes are waited by pidfd after a reload, so descriptors match exactly (UC-060).

## Capabilities

### Modified Capabilities
- `files`
- `reload`

## Out of scope

- Syncing finished file writes and update downloads to disk (open change `durable-file-writes`).

## Impact

Implemented by: [`c4444cf`](https://github.com/tilr-dev/tilrd/commit/c4444cf) Screens ahead of transfers, bounded streams, and Ctrl on any layout … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
