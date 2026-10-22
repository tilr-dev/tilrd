## MODIFIED Requirements

### Requirement: Input that is not wanted is ignored
`ExecInput` after `eof`, for an exec started without `stdin`, or for an exec that is not the client's MUST be ignored, the exec going on.

**Verified by**: `exec::input_that_is_not_wanted_is_ignored`.

#### Scenario: Input after eof
- **WHEN** a client sends `ExecInput` data after one with `eof`
- **THEN** the data is dropped and the program sees end of file as before
