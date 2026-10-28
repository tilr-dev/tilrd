## ADDED Requirements

### Requirement: Reload storm verification waits for queued work

The reload storm integration test SHALL deliver its burst at a controlled successor startup stage, after signal handlers are installed and before pane restoration completes. It SHALL distinguish that initialization from the reload queued by the burst, and SHALL wait for the queued reload's successor to resume before verifying the original shell. It SHALL use event-controlled pauses rather than elapsed time to establish this ordering.

**Verified by**: `reload::a_storm_of_reload_signals_costs_no_shell`.

#### Scenario: Signals queue a reload before pane restoration completes
- **WHEN** a burst of SIGUSR1 arrives after the successor installs its handlers but before it announces resume
- **THEN** the test does not treat that resume as completion of the queued reload
- **AND** it connects its verification client only after the queued reload's successor resumes, and observes the original shell and server pid
