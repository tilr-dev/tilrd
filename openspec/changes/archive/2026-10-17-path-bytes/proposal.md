# Proposal: Bytes twins for paths and names that are not UTF-8

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-071 was red: a file whose name is not UTF-8 could not be listed so that it could be read back (itests/USE_CASES.md).

## What Changes

- Every path in the Fs* requests has a `bytes` twin (`path_bytes`, `FsEntry.name_bytes` and others); the server goes by the bytes when given.
- The driver passes such names in JSON as Python surrogateescape.

## Capabilities

### Modified Capabilities
- `files`

## Out of scope

- A feature flag telling clients that the server honours the twins (open change `protocol-feature-flags`).

## Impact

Implemented by: [`c4444cf`](https://github.com/tilr-dev/tilrd/commit/c4444cf) Screens ahead of transfers, bounded streams, and Ctrl on any layout … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change). Additive proto fields; PROTOCOL_VERSION unchanged.
