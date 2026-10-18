# Proposal: itests groups F and G: live software across updates, transfers

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Updates and reloads had been tested with shells only; programs with alternate screens, open connections and transfers in flight were not. The group was written ahead of the fixes and left red: UC-064 (temp files left by a write cut by a reload), UC-065 (partial state files on a full disk), UC-071 (non-UTF-8 names), UC-072 (screen frames queued behind file reads), UC-073 (a stream buffered by packets, not bytes), UC-077 (Exec, not implemented), UC-079 (Ctrl+C on a non-Latin layout) (commit message).

## What Changes

- Panes with a shell, counter, vim, htop, less, a python REPL, sleep and an outbound TCP connection across updates from a fake CDN, rollbacks, a full disk and 50 reloads under load.
- Files, streams, SOCKS, clipboard, `tilr edit/open`, keys and mouse, a real `ssh -A` with `git clone`, Cloud tunnels, and Exec against a draft.
- tilr-driver gains file, clipboard, mouse, agent relay, SOCKS listener and exec commands.

## Capabilities

### Modified Capabilities
- `update`
- `reload`
- `files`
- `ssh-agent-forwarding`
- `exec`

## Out of scope

- Nothing recorded.

## Impact

Implemented by: [`e5ac639`](https://github.com/tilr-dev/tilrd/commit/e5ac639) Live software across updates and transfers (itests groups F and G, UC-060…07A) … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change). The red cases were fixed by `2026-10-17-bulk-lane-and-bounded-streams`, `2026-10-17-reload-safe-writes`, `2026-10-17-path-bytes`, `2026-10-17-physical-key-code` and `2026-10-17-exec`.
