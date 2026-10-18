## MODIFIED Requirements

### Requirement: State directory
What outlives a restart SHALL be kept in the state directory: `$TILR_STATE_DIR`, else `$XDG_STATE_HOME/tilr`, else `~/.local/state/tilr`, created 0700 (its parents as usual) when missing. At start the server SHALL set an existing state directory owned by its euid to 0700 and SHALL NOT use one owned by another user (`<dir> belongs to another user`). The server's id SHALL be the UUID in `server-id` there (0600), made on first start and written aside and hard-linked into place so concurrent starts agree; a server that cannot keep it SHALL warn and use an id that lasts until it stops. The token file kept there is the `access-control` spec's.

**Verified by**: itests `UC-005` (`uc_005.py::on_a_clean_system_tilr_makes_its_directories_private`), `auth::an_existing_state_directory_is_made_private`.

#### Scenario: Clean home
- **WHEN** a user with no `~/.local/state` runs `tilr new -d`
- **THEN** `~/.local/state/tilr` is 0700 and the files in it are 0600

#### Scenario: Loose state directory
- **WHEN** `~/.local/state/tilr` exists with mode 0755 and holds `server-id` and the token, and `tilrd` starts
- **THEN** the directory is 0700 and the server keeps its id and token
