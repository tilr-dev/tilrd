# Proposal: Finished file writes and installed releases are synced to disk

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #9): an `FsWrite` replaces the file without fsync of the file or its directory, and `update` does the same. Still so at 1163abd: `receive` writes the temporary file, `flush`es (which syncs nothing to disk), drops it and renames it over the target (`crates/tilrd/src/files.rs:611-615`) before the success `FsAck`; `tilrd update` writes the unpacked programs without a sync (`update.rs:199-210`) and renames them into place (`update.rs:157-161`) without syncing the directory, and the running server then reloads into the new file. A power loss after the success answer can leave an empty or old file, or an empty executable the next boot starts. The reload state files already sync before the rename (`reload.rs:412-433`, UC-065).

## What Changes

- A finished `FsWrite` syncs the temporary file (`fsync`) before the rename and the directory after it, and answers only then; a sync error is the write's error (the temporary file is removed, the target unchanged when the first sync fails).
- `tilrd update` syncs each unpacked program before the renames and the executable's directory after them, before it prints `updated …` and exits 0.
- One helper (write data, `sync_all`, rename, sync the directory) shared by both.

## Capabilities

### Modified Capabilities
- `files`: new requirement "A finished write is on disk before it is acknowledged" (the success answer comes after the data and the rename are on disk).
- `update`: new requirement "The installed release is on disk before success" (both programs and the renames are synced before success).

Changed while implementing: the sync was first written into the existing requirements ("Writes are whole or not at all", "The release is put in place by two renames, the server last"); that made their text longer than `openspec validate --strict` accepts, so it is a requirement of its own in each spec.

## Non-goals

- Proving durability with power cuts in tests (not reproducible in containers); the tests observe the sync calls.
- `F_FULLFSYNC` on macOS (plain `fsync`, as the state files).
- The reload state files and the directory sync for them (in the runtime directory, read by the successor in the same boot).
- `FsMkdir`, `FsRename`, `FsDelete` (metadata only; unchanged).

## Impact

`crates/tilrd/src/{files,update}.rs` (and a small shared helper), itests UC-046 and UC-060 cases, README "Files" and "Updating". Writes get slower by one or two syncs per file. No protocol change, no PROTOCOL_VERSION or feature change, nothing crosses a reload (a write cut by a reload is unchanged: the file stays as it was). An update from the previous release: the old `tilrd update` installs this one without syncing (as today); from then on updates sync. Not breaking.
