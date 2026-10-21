## ADDED Requirements

### Requirement: Missing-pane input receives a terminal acknowledgement
Numbered input from a greeted client with a stable identity naming no running pane SHALL receive `Error { message: "no such pane: <name as sent>" }` then `InputAck { pane_id: <name as sent>, seq: <input_seq> }`. Clients SHALL reclaim that exact pane string's queued input through the acknowledged sequence. Rejection MUST NOT change applied counters. Unnumbered requests SHALL receive only the error.

**Verified by**: `input_delivery::missing_pane_input_is_rejected_and_reclaimed_across_reconnect`.

#### Scenario: Rejected numbered input
- **WHEN** a stable identified client sends numbered key, mouse, paste, raw, scroll or resize input naming an absent UUID or malformed pane name
- **THEN** it receives the error and terminal acknowledgement, can reclaim that pane's queued input and keeps other panes' pending input
- **AND** the acknowledgement terminates delivery without reporting application to a PTY or changing a live pane's applied counter

#### Scenario: Lost rejection acknowledgement
- **WHEN** the rejection acknowledgement is lost and the client replays after reconnect or reload while the pane remains absent
- **THEN** the server returns the same error and acknowledgement without adding an applied counter to HelloAck

#### Scenario: Acknowledged rejection does not replay
- **WHEN** the client reconnects after consuming the rejection acknowledgement
- **THEN** its outbox no longer replays that rejected input
