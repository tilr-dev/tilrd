## ADDED Requirements

### Requirement: A finished write is on disk before it is acknowledged
Before an `FsAck` with an empty error the server SHALL have synced the temporary file's data to disk before the rename and the target's directory after it; a failed sync SHALL be the write's error.

**Verified by**: UC-046 (`uc_046.py::a_finished_write_is_synced_before_it_is_acknowledged`).

#### Scenario: Synced before the answer
- **WHEN** a client writes `~/notes.md` and the server's system calls are traced
- **THEN** an `fsync` of the temporary file comes before the `rename` onto `~/notes.md`, an `fsync` of the home directory after it, and both before the `FsAck`
