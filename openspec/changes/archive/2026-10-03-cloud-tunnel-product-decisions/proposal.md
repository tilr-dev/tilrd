# Proposal: Tunnels for users, abuse protection and the Cloudflare tenant plan

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Offering tunnels to users needed product and legal limits before any server code (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, 2026-10-03).

## What Changes

- A tunnel is pinned to the Tilr service only; attribution, a kill switch and terms of service are required.
- Cloudflare MSP / Tenant Platform as the route to per-user tunnels.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`

## Out of scope

- Server code (see `2026-10-09-cloud-tunnels`).
- The Cloud control plane itself (another repository).

## Impact

Implemented by: Pre-repository: reconstructed from Basic Memory note `main/projects/tilr/tilr-zhurnal-reshenii`; product decisions only, no code in this repository.
