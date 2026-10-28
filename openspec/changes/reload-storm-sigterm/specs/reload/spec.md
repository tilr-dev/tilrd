## ADDED Requirements

### Requirement: Reload storm verification waits for queued work

A dedicated regression SHALL distinguish successor initialization from the reload queued by SIGUSR1 during restoration. The periodic storm test SHALL end with an exec-fenced signal, acknowledged by its successor as held across exec. Both tests SHALL wait for the queued reload's successor to resume before verifying the original shell, using event-controlled pauses for ordering.

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
