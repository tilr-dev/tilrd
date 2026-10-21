## ADDED Requirements

### Requirement: Hello input probes are bounded
The server SHALL give pane processes five seconds to answer Hello input probes,
checked on each supervisor tick. A pane process that has not answered within that
bound SHALL be killed and its pane closed. Hello SHALL complete after those pane
links close, retaining the applied input counters from responsive panes. A client
that has sent a valid Hello SHALL NOT be closed by the timeout for missing Hello
while its probes are pending.

**Verified by**: `input_delivery::a_stuck_pane_does_not_block_other_panes_hello`.

#### Scenario: A pane stops reading its input pipe
- **WHEN** two stable clients send Hello while a pane process is stopped before
  answering its input probe
- **THEN** the unresponsive pane is closed within the probe bound
- **AND** both clients receive HelloAck with applied counters for responsive panes
- **AND** input on responsive panes continues without duplicate application
