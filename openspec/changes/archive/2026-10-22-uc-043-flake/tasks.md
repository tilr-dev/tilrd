# Tasks

## 1. Flake

- [x] 1.1 Explain and fix the UC-043 oracles: random proxy backoff does not bound p90; incoming frames can keep the client alive while uplink silence expires the server timeout. Replace random-condition bounds with metrics and recovery checks. Reproduce deterministically with fixed 3000 ms each-way delay and uplink freeze plus delayed downlink; the old assertions reject both valid states. Verify the controlled baseline/delay/recovery threshold, delayed recovery, and silence/reconnect scenarios without consecutive-run criteria.

## 2. Checks

- [x] 2.1 Run the required repository checks through the finish workflow.
