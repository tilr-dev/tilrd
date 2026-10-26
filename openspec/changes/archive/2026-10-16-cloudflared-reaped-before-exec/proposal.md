# Proposal: cloudflared is reaped before a reload's exec

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

After the supervisor re-exec'd on a reload the old `cloudflared` stayed a zombie of the new server; on Linux `kill -0` still sees a zombie, and the reload test flapped there (commit 7210b2c). The itests branch found and fixed the same defect independently (e6885b0, Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- The tunnel task owns the child: stopping it sends SIGTERM, waits up to 3 s, then SIGKILL, and returns once the child is reaped; the supervisor awaits that before the exec and on shutdown.
- Test fixtures use a fake Cloudflare account id.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`
- `reload`

## Out of scope

- A `cloudflared` stuck past the wait, and one left after a SIGKILL of the server (open change `cloudflared-outlives-exec`).

## Impact

Implemented by: [`7210b2c`](https://github.com/tilr-dev/tilrd/commit/7210b2c) fix(cloud): reap cloudflared on stop so a reload leaves no zombie (2026-10-16); [`e6885b0`](https://github.com/tilr-dev/tilrd/commit/e6885b0) The server is tilrd … (2026-10-17). The two commits duplicate the fix on `main` and `itests`; the merge of `itests` keeps one (review finding #1).
