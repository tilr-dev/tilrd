# Isolate reload compatibility tests from launch-policy queues

## Why

Real successor launches in parallel reload tests can spend over ten seconds in
Gatekeeper's serialized scan queue before executing user code, even with the
existing warmed executable fixture. This consumes the entire production probe
deadline before the successor can report its format or command-line verdict.

## What Changes

- Record measured launch-policy queue latency from actual failed probes.
- Isolate OS startup admission from compatibility assertions in the test stand.
- Preserve the execution deadline after startup and verify both phases with
  controlled holds.
- Keep the production ten-second probe and successor resume-check limits.

## Capabilities

- Add a test-stand requirement for isolating compatibility verdicts from the
  measured startup delay.
