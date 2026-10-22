## MODIFIED Requirements

### Requirement: Input is acknowledged after application
Clients SHALL number input per identity and pane and retain it until ACK for ordered replay. The server SHALL ACK completed PTY writes independently of output, deduplicate accepted sequences and report applied counters in Hello. Deduplication ends at finalization, one hour without reconnect after disconnect, or pane exec; later replay MAY duplicate input. Delivery and latency numbering SHALL be independent. Outboxes SHALL reject overflow before sending or advancing counters.

**Verified by**: `input_delivery::numbered_input_requires_a_stable_client_id`, `input_delivery::disconnect_before_application_replays_one_copy`, `input_delivery::applied_input_with_a_lost_ack_is_not_applied_twice`, `input::tests::overflow_is_atomic_and_ack_reclaims_capacity`, `client::attach_replays_a_key_when_the_connection_ends_without_an_ack`, `session::input_delivery_tests::reconnect_replays_only_unacknowledged_input_in_order`.

#### Scenario: Application without screen output
- **WHEN** input is written to a PTY whose program emits no output
- **THEN** the client receives an InputAck and can remove that input from its queue

#### Scenario: Lost acknowledgement
- **WHEN** a connection ends after input is applied but before its acknowledgement arrives, reconnects within one hour, and no pane exec occurred
- **THEN** HelloAck reports the applied sequence and replay cannot apply the input again

#### Scenario: Pending duplicate
- **WHEN** a frame is replayed while some of its bytes are still pending
- **THEN** only the original pending bytes are written and the frame is acknowledged after their final byte

#### Scenario: Numbered input without a stable identity
- **WHEN** a greeted client without `Hello.client_id` sends input with a nonzero `input_seq`
- **THEN** the server returns `Error "numbered input requires a stable client_id"` and does not apply that input

#### Scenario: Full outbox
- **WHEN** another input would exceed 4 MiB or 4096 queued frames
- **THEN** admission fails visibly, the frame is not sent and existing queued input remains available for replay

## ADDED Requirements

### Requirement: Input delivery identities have a bounded lifetime
Clients SHALL finalize only after all admitted input is acknowledged. The server SHALL remove finalized identities immediately and disconnected identities after one hour without reconnect. Reconnect before expiry SHALL cancel the deadline. Cleanup SHALL preserve queued PTY bytes. Neither delivery counters nor disconnect deadlines SHALL survive exec. Protocol version SHALL remain 1.

**Verified by**: `input_delivery::finalized_sessions_do_not_accumulate_delivery_history`, `input_delivery::expired_sessions_are_removed_but_active_sessions_are_retained`, `input_delivery::expiry_preserves_pending_bytes_without_stale_ack_markers`, `input_delivery::clients_cannot_forge_internal_delivery_cleanup`, `supervisor::tests::input_retention_expires_at_one_hour_and_reconnect_cancels_it`, `client::normal_detach_finalizes_its_acknowledged_input`, `session::input_delivery_tests::normal_disconnect_drains_input_before_finalizing`.

#### Scenario: Many completed client sessions
- **WHEN** successive identities stop admitting input, receive all acknowledgements, send `FinalizeInput` and close
- **THEN** their delivery records SHALL be removed immediately, further numbered input on the finalized connection SHALL be refused, and later Hello SHALL report no applied input for them

#### Scenario: Disconnect retention and expiry
- **WHEN** an identity disconnects without finalizing
- **THEN** reconnect before one hour retains its applied counter and cancels expiry; reconnect after an uninterrupted hour starts with no applied counter and may replay already applied input

#### Scenario: Pending bytes outlive history
- **WHEN** an expired identity has bytes still queued for its PTY
- **THEN** cleanup preserves the bytes and the pane keeps running when they are written

#### Scenario: Client cannot request another identity's cleanup
- **WHEN** a client supplies internal cleanup fields
- **THEN** ingress SHALL clear them and SHALL NOT remove any other identity
