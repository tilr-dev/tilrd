## MODIFIED Requirements

### Requirement: The release is put in place by two renames, the server last
The update SHALL rename the unpacked `tilr` over `tilr` in the executable's directory first and then the unpacked `tilrd` over the executable (under whatever file name it has), so a server started from it notices and reloads (see the `reload` spec); the update restarts nothing. Both unpacked programs SHALL be synced to disk before the first rename and the directory after the second. On success it SHALL print `updated <old> -> <new>: <directory>` and `a running server reloads itself and keeps its shells` and exit 0, leaving only `tilrd` and `tilr` of the release.

**Verified by**: `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`, itests `UC-060` (`uc_060.py::an_update_is_synced_before_it_reports_success`).

#### Scenario: Update with a pane open
- **WHEN** a server with a pane holding `KEPT=yes` runs `tilrd update` against a CDN with a newer release
- **THEN** stdout says `-> 9.9.9-test`, the executable and the `tilr` next to it are the release's, the server logs `executable changed on disk`, and after reconnecting `test "$KEPT" = yes` exits 0

#### Scenario: Synced before success
- **WHEN** `tilrd update` installs a newer release and its system calls are traced
- **THEN** each unpacked program is `fsync`ed before the first `rename`, the executable's directory is `fsync`ed after the second, and both before `updated` is printed
