# Proposal: Handshake timeout, peer uid on host and agent sockets, private state directory

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests group D (UC-034 … UC-039) checked sockets against other users and silent connections; a slowloris or a thousand silent connections had to leave the server working (itests/USE_CASES.md; Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- A handshake has 10 s; one not finished is closed (UC-038, UC-039).
- The host and agent sockets check the peer's uid (UC-035).
- Dead agent sockets are swept at start; a socket someone listens on stays (UC-036).
- The state directory is 0700, `server-id` 0600.

## Capabilities

### Modified Capabilities
- `access-control`
- `ssh-agent-forwarding`
- `daemon-lifecycle`

## Out of scope

- A deadline for `Hello` after the WebSocket handshake (open change `hello-deadline`).
- Tightening an existing state directory (open change `service-install-ownership`).

## Impact

Implemented by: [`13f9522`](https://github.com/tilr-dev/tilrd/commit/13f9522) tilrd listens on 127.0.0.1:7890 and the unix socket by default … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
