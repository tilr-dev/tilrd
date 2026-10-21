## ADDED Requirements

### Requirement: Input is acknowledged after application
Clients SHALL number input per stable identity and pane, retain it until acknowledged, and replay it after reconnecting. The server SHALL acknowledge completed PTY writes independently of output, discard accepted duplicates, and report applied counters during Hello. Delivery and latency numbering SHALL be independent. Outboxes SHALL reject overflow before sending or advancing delivery counters.

**Verified by**: `input_delivery::disconnect_before_application_replays_one_copy`, `input_delivery::applied_input_with_a_lost_ack_is_not_applied_twice`, `input_delivery::partially_written_input_keeps_its_boundary_through_exec`, `input::tests::overflow_is_atomic_and_ack_reclaims_capacity`, `client::attach_replays_a_key_when_the_connection_ends_without_an_ack`, `session::input_delivery_tests::reconnect_replays_only_unacknowledged_input_in_order`.

#### Scenario: Application without screen output
- **WHEN** input is written to a PTY whose program emits no output
- **THEN** the client receives an InputAck and can remove that input from its queue

#### Scenario: Lost acknowledgement
- **WHEN** a connection ends after input is applied but before its acknowledgement arrives
- **THEN** HelloAck reports the applied sequence and replay cannot apply the input again

#### Scenario: Pending duplicate
- **WHEN** a frame is replayed while some of its bytes are still pending
- **THEN** only the original pending bytes are written and the frame is acknowledged after their final byte

#### Scenario: Full outbox
- **WHEN** another input would exceed 4 MiB or 4096 queued frames
- **THEN** admission fails visibly, the frame is not sent and existing queued input remains available for replay

## MODIFIED Requirements

### Requirement: Version 3 is one contract
A server speaking version 3 SHALL implement, without a feature flag, `Ping`, `Hello.client_id` with superseding and close code 4000, close codes 4001 and 4002, the bulk window paced by pongs, `KeyInput.code`, the `*_bytes` twins of file paths, `PaneInfo.viewed_by` / `used_at_ms`, and acknowledged input with replay; none of them SHALL appear in `HelloAck.features`.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`; each part is verified by the requirement that specifies it.

#### Scenario: A v3 client relies on the twins
- **WHEN** a client greeted with `protocol_version` 3 names a file by `path_bytes`
- **THEN** the server goes by the bytes, and `HelloAck.features` never lists `path-bytes`, `client-id`, `key-code`, `ping` or input delivery
