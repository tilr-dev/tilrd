## ADDED Requirements

### Requirement: An existing state directory is made private
At start the server SHALL set an existing state directory owned by its euid to 0700 and SHALL NOT use one owned by another user (`<dir> belongs to another user`), before it reads or writes `server-id`, the token or the Cloud link there.

**Verified by**: `auth::an_existing_state_directory_is_made_private`.

#### Scenario: Loose state directory
- **WHEN** the state directory exists with mode 0755 and holds `server-id` and the token, and `tilrd` starts
- **THEN** the directory is 0700 and the server keeps its id and token
