## ADDED Requirements

### Requirement: tilr profiles lists the server's profiles
`tilr profiles` SHALL call `pane.profiles` on its target and print one line per profile: the name, `sandbox` or `-`, and the description, or `error: <reason>` for an invalid profile; it SHALL exit 0 when the list was received, also when it is empty.

**Verified by**: `client::tilr_profiles_lists_the_profiles_of_the_server`.

#### Scenario: One valid, one invalid
- **WHEN** the server has a valid `demo` with `description = "a demo"` and an invalid `bad`
- **THEN** `tilr profiles` prints a line with `bad` and `error: `, then a line `demo  -  a demo`, and exits 0
