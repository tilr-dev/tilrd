# Proposal: Fix the itests uc_043 flake

## Why

The UC-043 random-network checks impose timing and disconnect-order guarantees
that the transport and server do not provide. Controlled network conditions can
reject correct server behavior without any server overload.

## What Changes

- Explain the random proxy backoff and asymmetric silence mechanisms.
- Replace random-network timing oracles with diagnostic metrics and recovery
  assertions, retaining deterministic echo-threshold and liveness checks.
- Reproduce both rejected states with controlled delay and an uplink freeze.

## Capabilities

None: tests only, no requirement changes.

## Impact

`itests/scenarios/uc_043.py` and its documentation. No server behavior, protocol,
reload or update impact.
