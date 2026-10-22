## MODIFIED Requirements

### Requirement: The release is put in place by two renames, the server last
The update SHALL rename the unpacked `tilr` over `tilr` in the executable's directory first and then the unpacked `tilrd` over the executable (under whatever file name it has), then SHALL explicitly request reload from a running server started from it (see the `reload` spec); the update SHALL NOT start a server if none is running. On success it SHALL print `updated <old> -> <new>: <directory>` and `a running server reloads itself and keeps its shells` and exit 0, leaving only `tilrd` and `tilr` of the release.

**Verified by**: `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: Update with a pane open
- **WHEN** a server with a pane holding `KEPT=yes` runs `tilrd update` against a CDN with a newer release
- **THEN** stdout says `-> 9.9.9-test`, the executable and the `tilr` next to it are the release's, the server logs an explicit reload request, and after reconnecting `test "$KEPT" = yes` exits 0


## Purpose

Covers release discovery, verification and atomic installation by `tilrd update` / `tilr update`, followed by an explicit request to reload a running server. Reload handover is specified by `reload`.
