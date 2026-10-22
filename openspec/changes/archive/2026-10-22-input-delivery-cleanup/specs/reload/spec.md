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

### Requirement: Client connections end and clients reconnect
A reload SHALL end every client connection (sockets are not carried over); a client that reconnects SHALL find its panes as they were. Input admitted to the client's delivery queue SHALL reach the pane after reconnection, including input the supervisor had not read before exec. Admitted input SHALL retain its order without loss; replay MAY duplicate input applied but not acknowledged before reload.

**Verified by**: `input_delivery::unread_input_is_replayed_after_reload_in_the_middle_of_a_batch`; itests `UC-063` (`uc_063.py::a_client_typing_through_an_update_reconnects_and_nothing_typed_is_lost_or_doubled`).

#### Scenario: Typing through an update
- **WHEN** a client types numbered lines into `cat >>typed` through an update
- **THEN** after reconnection every admitted line is in the file in order, allowing replay of input not acknowledged before reload
