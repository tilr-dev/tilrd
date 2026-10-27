# Proposal: Black-box integration tests on Dagger (itests) and tilr-driver

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The cargo tests drive one machine; real failure modes (SSH sessions dropping, half-open connections, live software across an update) needed real OS containers and a network that can be broken. Decided 2026-10-16: the driver lives in this repository, everything under `itests/`, a proxy of our own, the service optional and checked by its files only (itests/USE_CASES.md; Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- `itests/` runs the server in Linux containers on the pb Dagger engine (`./dt`), the cargo tests one per container, clippy and fmt, and scenarios from itests/USE_CASES.md; PASS / FAIL / BLOCKED reports in `itests/results/`.
- `crates/tilr-driver`: a scripted client (JSON lines on stdin and stdout, one connection per scenario, reconnect, screen as text).
- `itests/tt/proxy.py` freezes one or both directions, resumes, cuts with RST, delays, cuts after n WebSocket messages.
- First scenarios: UC-010 (red: no `tilr` client yet), UC-013, UC-040, UC-060. Result of the first full run: 201 PASS / 3 FAIL.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`
- `client-liveness`
- `update`

## Out of scope

- Load and security thresholds, other distributions, a macOS backend (open change `itests-stage-5`).

## Impact

Implemented by: [`a92d1fe`](https://github.com/tilr-dev/tilrd/commit/a92d1fe) Black-box integration tests on Dagger … (2026-10-16). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
