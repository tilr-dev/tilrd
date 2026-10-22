## MODIFIED Requirements

### Requirement: Pane input and output are not lost across a reload
Output a pane prints during a reload SHALL reach the client after it reconnects. Queued PTY input bytes and latency marks SHALL survive exec. Delivery counters and pending delivery acknowledgement boundaries SHALL NOT survive exec: each pane starts with an empty delivery map. A client replaying input applied but not acknowledged before reload MAY duplicate that input, including an already written prefix of a partially written frame. This is an accepted risk of a manually requested reload.

**Verified by**: `reload::output_printed_during_a_reload_is_not_lost`, `reload::input_typed_just_before_a_reload_reaches_the_shell`, `metrics::the_last_input_and_input_still_waiting_survive_a_reload`, `input_delivery::exec_resets_delivery_history_and_preserves_pending_bytes`.

#### Scenario: Input just before
- **WHEN** a client types a command and the server reloads at once
- **THEN** queued bytes reach the shell; replay may duplicate bytes that were not acknowledged before reload

#### Scenario: Reload during a batch
- **WHEN** a pane reloads with part of an input frame still waiting for the PTY and the client replays it
- **THEN** the restored pending suffix is preserved, Hello reports no applied counter, and replay may duplicate the already written prefix or queued suffix
