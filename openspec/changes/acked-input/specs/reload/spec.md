## MODIFIED Requirements

### Requirement: Client connections end and clients reconnect
A reload SHALL end every client connection (sockets are not carried over); a client that reconnects SHALL find its panes as they were. Input admitted to the client's delivery queue SHALL reach the pane exactly once after reconnection, including input the supervisor had not read before exec.

**Verified by**: `input_delivery::unread_input_is_replayed_after_reload_in_the_middle_of_a_batch`; itests `UC-063` (`uc_063.py::a_client_typing_through_an_update_reconnects_and_nothing_typed_is_lost_or_doubled`).

#### Scenario: Typing through an update
- **WHEN** a client types numbered lines into `cat >>typed` through an update
- **THEN** after reconnection every admitted line is in the file once, in order

### Requirement: Pane input and output are not lost across a reload
Output a pane prints during a reload SHALL reach the client after it reconnects. Input SHALL reach the shell exactly once with its latency marks kept. Applied counters, accepted counters and pending byte boundaries SHALL survive exec so replay neither loses nor duplicates partially written input.

**Verified by**: `reload::output_printed_during_a_reload_is_not_lost`, `reload::input_typed_just_before_a_reload_reaches_the_shell`, `metrics::the_last_input_and_input_still_waiting_survive_a_reload`, `input_delivery::partially_written_input_keeps_its_boundary_through_exec`.

#### Scenario: Input just before
- **WHEN** a client types a command and the server reloads at once
- **THEN** the shell runs the command once

#### Scenario: Reload during a batch
- **WHEN** a pane reloads with part of an input frame still waiting for the PTY and the client replays it
- **THEN** the restored pending suffix is written exactly once and the applied sequence advances only after its final byte
