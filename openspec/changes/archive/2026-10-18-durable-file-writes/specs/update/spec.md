## ADDED Requirements

### Requirement: The installed release is on disk before success
Both unpacked programs SHALL be synced to disk before the first rename and the executable's directory after the second, before the update prints `updated`.

**Verified by**: itests `UC-060` (`uc_060.py::an_update_is_synced_before_it_reports_success`).

#### Scenario: Synced before success
- **WHEN** `tilrd update` installs a newer release and its system calls are traced
- **THEN** each unpacked program is `fsync`ed before the first `rename`, the executable's directory is `fsync`ed after the second, and both before `updated` is printed
