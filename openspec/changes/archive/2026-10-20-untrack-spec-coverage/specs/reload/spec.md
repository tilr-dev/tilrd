## MODIFIED Requirements

### Requirement: Tests explicitly release paused failpoints
With `failpoints`, each `name=pause` hit configured by `TILR_FAILPOINTS` or `TILR_FAILPOINTS_FILE` SHALL open a new Unix stream connection to `TILR_FAILPOINTS_SOCKET`, send its name and a newline, and wait for byte `r`. Time SHALL NOT release a point. Missing socket configuration, I/O failure, EOF or another byte SHALL fail the point. Without `failpoints`, these variables SHALL have no effect.

**Verified by**: `reload::a_successor_is_checked_while_the_server_serves_and_then_reloaded_into`, `reload::a_runtime_named_pause_holds_a_pane_check_until_released`, `reload::a_disconnected_pause_controller_refuses_the_reload`.

#### Scenario: Pause a successor check
- **WHEN** the test receives `supervisor-resume` on its control socket
- **THEN** the original server still serves its client while the check waits; sending `r` allows the reload and the same shell survives

#### Scenario: Runtime pane name and repeated hits
- **WHEN** the test writes `pane-resume:<pane id>=pause` to the failpoints file after creating the pane and triggers a reload
- **THEN** the named check waits for its own release and the pane resumes successfully; a later reload waits on a fresh connection with no stale release and resumes the pane again after release

#### Scenario: Controller disappears
- **WHEN** the test closes a paused successor's connection without sending `r`
- **THEN** the check fails and the original server keeps serving

#### Scenario: The async runtime keeps serving
- **WHEN** an async caller needs the runtime to keep serving while a failpoint is paused
- **THEN** it SHALL run the blocking point on a blocking thread and await it
