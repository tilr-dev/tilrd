## ADDED Requirements

### Requirement: Reload storm verification waits for queued work

A dedicated queued-reload regression test SHALL deliver its burst at a controlled successor startup stage, after signal handlers are installed and before pane restoration completes. It SHALL distinguish that initialization from the reload queued by the burst, and SHALL wait for the queued reload's successor to resume before verifying the original shell. It SHALL use event-controlled pauses rather than elapsed time to establish this ordering. The separate periodic storm test SHALL finish its traffic before establishing an exec fence with all old-image signal threads held. A final SIGUSR1 sent at that fence SHALL be acknowledged by the successor as held across exec, and verification SHALL wait for the reload queued by that acknowledgment.

**Verified by**: `reload::signals_during_restoration_wait_for_the_queued_reload`, `reload::a_storm_of_reload_signals_costs_no_shell`.

#### Scenario: Signals queue a reload before pane restoration completes
- **WHEN** a burst of SIGUSR1 arrives after the successor installs its handlers but before it announces resume
- **THEN** the test does not treat that resume as completion of the queued reload
- **AND** it connects its verification client only after the queued reload's successor resumes, and observes the original shell and server pid

#### Scenario: Periodic storm completion fences its final signal across exec
- **WHEN** periodic SIGUSR1 traffic has ended and the supervisor reaches an event-controlled pause immediately before exec with signals held on all its threads
- **AND** the test sends a final SIGUSR1 and releases the pause
- **THEN** the successor records that held signal before its initialization resume
- **AND** the test waits for the additional reload queued by that signal before verifying the original shell
- **AND** buffered receipts from earlier storm signals cannot satisfy the fence
