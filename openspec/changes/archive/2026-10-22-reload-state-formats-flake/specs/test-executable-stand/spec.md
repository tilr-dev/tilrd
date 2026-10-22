## ADDED Requirements

### Requirement: Compatibility verdicts can await controlled successors
With `failpoints`, selected reload compatibility tests SHALL be able to await
`state-formats` completion independently of the production probe deadline.
Without an explicit test opt-in the ten-second deadline SHALL still apply.
The successor resume-check deadline SHALL remain unchanged.

**Verified by**: `reload::tests::delayed_state_formats_obeys_the_selected_policy`.

#### Scenario: Delayed successor in a compatibility test
- **WHEN** a successor is held until virtual time passes the production deadline
- **THEN** the opted-in check remains pending and reads its formats after release
- **AND** the same held successor without the opt-in is refused with a probe timeout
