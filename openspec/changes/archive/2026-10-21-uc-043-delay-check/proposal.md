# Deterministic UC-043 echo delay check

## Why

UC-043 needs a controlled network condition that puts input echo p90 above five
seconds, alongside a healthy baseline, independently of random loss and jitter.

## What Changes

- Add an isolated UC-043 case using the existing proxy with fixed bidirectional delay.
- Check complete echo samples before, during and after the delay without sleeps,
  repeat runs or increased timeouts; document the case and proxy setting.

## Capabilities

### New Capabilities
- `itest-echo-delay`: deterministic input echo threshold verification.

## Impact

Only the new UC-043 scenario and itests documentation change. Existing scenarios
and the server remain unchanged.
