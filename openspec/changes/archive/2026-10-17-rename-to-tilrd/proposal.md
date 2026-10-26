# Proposal: The server is tilrd

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Decided 2026-10-16: the daemon is `tilrd` (like sshd) and `tilr` is the client, in a tmux-style model (Basic Memory `main/projects/tilr/tilrd-naming-install-and-run-model`). Dan chose no backward compatibility (early development, no users), but an old `tilr-server` still has to update in place (UC-06A) (`main/projects/tilr/tilrd-idei-read-only-zritel-client-id`, `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- The crate, directory (`crates/tilrd`), binary, clap name and metrics (`tilrd_*`) are named tilrd. **BREAKING** for metric names (external dashboards).
- A release ships the same file as `tilrd-<platform>` and `tilr-server-<platform>`, so an old `tilr-server` updates in place under its own name, and `tilrd update` falls back to the old name to roll back (UC-06A).
- itests build exactly the source they are given (`tt-fresh` re-stamps changed files against a manifest of hashes on the shared target caches); the scroll test waits for the prompt.

Later: the dual-name transition (both names published, the old-name fallback in `tilrd update`, UC-06A) was removed by `2a8b425` (archive `2026-10-18-release-archive`), with no compatibility for old updaters, and the UC-06A case by `3ba0e93`.

## Capabilities

### Modified Capabilities
- `release-process`
- `update`
- `metrics`

## Out of scope

- tilr-mac, cloud, docs and installer renames (open change `tilr-mac-rename-compatibility`).

## Impact

Implemented by: [`e6885b0`](https://github.com/tilr-dev/tilrd/commit/e6885b0) The server is tilrd … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change). The cloudflared reaping in the same commit is in `2026-10-16-cloudflared-reaped-before-exec`.
