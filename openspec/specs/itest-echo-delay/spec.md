# itest-echo-delay Specification

## Purpose
UC-043 needs a controlled network condition that puts input echo p90 above five
seconds, alongside a healthy baseline, independently of random loss and jitter.

## Requirements

### Requirement: Controlled input echo threshold check
The integration suite SHALL provide an isolated UC-043 case using the existing
proxy to verify input-to-frame echo p90 against a five-second threshold. It SHALL
collect complete samples with zero delay, with fixed 3000 ms delay in each
direction and no jitter or loss, and after removing the delay. It SHALL synchronize
on echoed input without sleeps, repeat runs or increased timeouts.

**Verified by**: `uc_043.py::fixed_network_delay_moves_echo_p90_across_five_seconds`.

#### Scenario: Delay moves echo across the threshold
- **WHEN** the case measures echo with zero delay, then 3000 ms in both directions
- **THEN** the first p90 is below 5000 ms and the delayed p90 is above 5000 ms
- **AND** every measured input has a corresponding frame and the client stays connected

#### Scenario: Removing delay restores echo
- **WHEN** the case removes the fixed delay and collects a fresh sample
- **THEN** echo p90 is below 5000 ms again
- **AND** other cases retain their existing network settings
