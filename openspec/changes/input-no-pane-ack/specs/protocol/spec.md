## ADDED Requirements

### Requirement: Missing-pane input receives a terminal acknowledgement
For a greeted client with a stable identity, numbered pane input naming no running pane SHALL receive the existing `Error { message: "no such pane: <name as sent>" }` followed by `InputAck { pane_id: <name as sent>, seq: <input_seq> }`. This acknowledgement SHALL terminate delivery of the rejected input and allow the client to reclaim its outbox entries through that sequence for that exact pane string. It MUST NOT report application to a PTY or change any live pane's applied counters. This applies to key, mouse, paste, raw, scroll and resize input, including malformed pane names. Unnumbered requests SHALL retain their existing error-only behavior.

**Verified by**: `input_delivery::missing_pane_input_is_rejected_and_reclaimed_across_reconnect`.

#### Scenario: Rejected numbered input
- **WHEN** a stable identified client sends numbered input naming no running pane
- **THEN** it receives the error and terminal acknowledgement, can reclaim that pane's queued input and keeps other panes' pending input

#### Scenario: Lost rejection acknowledgement
- **WHEN** the rejection acknowledgement is lost and the client replays after reconnect or reload while the pane remains absent
- **THEN** the server returns the same error and acknowledgement without adding an applied counter to HelloAck

#### Scenario: Acknowledged rejection does not replay
- **WHEN** the client reconnects after consuming the rejection acknowledgement
- **THEN** its outbox no longer replays that rejected input
