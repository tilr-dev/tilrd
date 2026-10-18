# Proposal: itests group E: bad network and reconnect

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Reconnect behaviour on real bad networks had only been reasoned about (Basic Memory `main/projects/tilr/tilr-stendy-dlia-testirovaniia-reconnect-na-plokhoi-seti`). The group was written first and left red where the server fell short: UC-041 (a client that sends but no longer receives was never dropped) and UC-049 (a slow client was logged as dropped but stayed connected).

## What Changes

- Proxy as a service of its own: freeze, one-way drop, delay, held chunks, rate limits, RST, cut after N messages.
- netem, iptables and `ss -K` on client hosts with NET_ADMIN; a second host for a new address.
- tilr-driver: keepalive modes, an echo agent, streams, SOCKS, file writes, input latency and frame traces.
- Scenarios UC-041 … UC-051.

## Capabilities

### Modified Capabilities
- `client-liveness`

## Out of scope

- QUIC/UDP transport (deferred; open change `quic-transport`).

## Impact

Implemented by: [`5e936c2`](https://github.com/tilr-dev/tilrd/commit/5e936c2) Bad network and reconnect scenarios (itests group E, UC-041…051) … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change). The red cases were fixed by `2026-10-17-client-liveness-and-supersede`.
