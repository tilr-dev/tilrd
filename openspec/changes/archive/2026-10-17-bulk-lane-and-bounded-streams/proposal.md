# Proposal: Screens ahead of transfers; bounded bulk data

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-072 was red (echo up to 1.6 s during reads, up to 5 s both ways) and UC-073 was red (a stream buffered up to 64 MB) (itests/USE_CASES.md).

## What Changes

- The client's outbox has an interactive lane and a bulk lane (`FsData`, `StreamData`); bulk is taken only when the first is empty.
- Bulk is capped at 4 MiB per client and 1 MiB unread by the client, known from pongs to pings that carry the position (every 128 KiB); clients must answer pings.
- File reads encode their own chunks; the agent pump waits for room, so a slow client holds the program back.
- itests build the driver in release (a debug driver reads about 30 MB/s).

## Capabilities

### Modified Capabilities
- `client-liveness`
- `files`
- `ssh-agent-forwarding`

## Out of scope

- A bulk window stalled by a client that answers only the latest ping (open change `bulk-window-stall`).

## Impact

Implemented by: [`c4444cf`](https://github.com/tilr-dev/tilrd/commit/c4444cf) Screens ahead of transfers, bounded streams, and Ctrl on any layout … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change). The same commit also holds `2026-10-17-reload-safe-writes`, `2026-10-17-path-bytes` and `2026-10-17-physical-key-code`.
