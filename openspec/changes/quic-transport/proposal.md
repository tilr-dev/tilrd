# Proposal: A QUIC/UDP transport for bad networks

## Why

itests UC-043 (30-40% loss, 500 ± 200 ms) shows p50 echo delays of 1-13 s, accepted on 2026-10-17 as TCP retransmits; a QUIC/UDP transport was deferred as "отдельная задача потом" (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- To be designed: QUIC as a second transport carrying the same packets (one stream per lane, so bulk does not block frames), with the same Hello and token.

## Capabilities

### Modified Capabilities
- `protocol`
- `client-liveness`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Replacing the WebSocket transport.
- Directory sync (change `directory-sync`).

## Impact

New listener, connection type and feature flag; a reload must hand the UDP socket over like the TCP listener. Not breaking (additive transport).
