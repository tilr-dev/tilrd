## MODIFIED Requirements

### Requirement: tilr update runs the update in the local server
`tilr update [--cdn URL]` (or `TILR_CDN`) SHALL run `tilrd update [--cdn URL]` in the local server as a server command (a child of the server, in its environment; see the `exec` spec) and SHALL refuse a remote target with `server commands run in the local tilrd; on another machine, run tilr there`.

**Verified by**: `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload`.

#### Scenario: Remote target
- **WHEN** `tilr --connect ws://host:7890 update` runs
- **THEN** it fails with `server commands run in the local tilrd; on another machine, run tilr there`

### Requirement: tilr update waits for the reloaded server
When the update command exits 0, or ends with reason `reload` because the server reloaded into it, `tilr update` SHALL reconnect for up to 15 s and print `tilrd <version> is running`, else fail with `the server did not come back after the update: ...`. Another exit code SHALL be `tilr`'s exit status.

**Verified by**: `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload`.

#### Scenario: Update from a shell
- **WHEN** the user runs `tilr update` and a newer release is installed
- **THEN** the server reloads and `tilr` prints `tilrd <new version> is running` and exits 0
