# Proposal: Finished file writes and update downloads are synced to disk

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #9): an `FsWrite` replaces the file without fsync of the file or its directory (`files.rs:611-615`), and `update` does the same (`update.rs:104-121`). A power loss after the success answer can leave an empty or old file. State files already do temp + fsync + rename (UC-065).

## What Changes

- A finished write fsyncs the temp file before the rename and the directory after it, and answers only then.
- `tilrd update` fsyncs the downloaded executable and its directory before reporting success and reloading.

## Capabilities

### Modified Capabilities
- `files`: see `specs/files/spec.md`.
- `update`: see `specs/update/spec.md`.

## Out of scope

- Measuring durability with power cuts in itests (not reproducible in containers).

## Impact

`crates/tilrd/src/{files,update}.rs`. Slightly slower writes. Nothing crosses a reload; no protocol change.
