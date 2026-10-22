## ADDED Requirements

### Requirement: Compatibility probes distinguish launch admission from execution
With `failpoints`, explicitly opted-in compatibility probes SHALL wait for a
successor startup event before starting the ten-second execution deadline.
Execution, stdout collection, and exit SHALL remain bounded by that deadline.
Ordinary probes and builds without `failpoints` SHALL retain the ten-second
limit including startup. The successor resume-check deadline SHALL remain
unchanged.

**Verified by**: `reload::tests::delayed_state_formats_obeys_the_selected_policy`.

#### Scenario: Launch admission exceeds the execution deadline
- **WHEN** a test holds a successor before its startup event past ten seconds
- **THEN** the opted-in probe remains pending
- **AND** after startup and release it reads the successor formats successfully
- **AND** the same held successor without the opt-in times out

#### Scenario: Successor stalls after startup
- **WHEN** the opted-in successor signals startup and then stops making progress
- **THEN** the probe times out after the existing ten-second execution deadline
