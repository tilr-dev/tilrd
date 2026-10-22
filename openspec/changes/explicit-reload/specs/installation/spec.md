## MODIFIED Requirements

### Requirement: A new release reloads the running server
Run with a new release, the installer SHALL rename the new `tilr` into place first and the new `tilrd` over the old one last, then SHALL explicitly request reload from the running server so it reloads into it in place (same pid, panes, shells and their state; see the `reload` spec), and SHALL say `the running tilrd reloads itself and keeps its shells` when a `tilrd` of the user runs.

**Verified by**: itests `UC-003` (`uc_003.py::installing_a_new_release_reloads_the_running_server_and_keeps_its_shells`).

#### Scenario: New release over a running server
- **WHEN** a newer release is published and the installer runs
- **THEN** stderr says `downloading tilr <new version>` and `the running tilrd reloads itself and keeps its shells`, both executables report the new version, and the server pid, panes, `$KEPT`, `$$` and background job are the same

