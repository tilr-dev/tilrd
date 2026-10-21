## Terminal rejection

All six numbered pane-input variants pass a shared missing-pane guard after Hello and stable-identity validation. The supervisor sends the existing no-such-pane Error followed by InputAck naming the exact supplied pane string and delivery sequence. This acknowledgement means the rejected input is terminal, not that bytes were applied. No pane's applied counter or HelloAck applied-input list is changed.

The cumulative acknowledgement is safe because previous input for the same missing pane cannot be applied later: pane creation generates a fresh UUID rather than accepting a chosen ID. Invalid names cannot identify a pane either. The client can reclaim those entries using its existing outbox acknowledgement path; errors alone never discard input for live panes.

## Reconnect, reload and bounds

There is no new retained server state. If a rejection acknowledgement is lost, replay encounters the same absent pane and receives Error plus InputAck again. Once acknowledged, that entry no longer consumes the client's 4096-frame/4 MiB outbox. Both responses use the existing control lane; no bulk data or new unbounded queue is introduced. Unnumbered requests retain their existing behavior.

## Verification

An integration test uses a Pong as an ordered supervisor barrier. It checks all six input types, absent UUIDs and malformed names, outbox reclamation, repeated rejection after a lost acknowledgement, and an empty replay after acknowledgement. Live-pane pending entries must remain queued. Existing identity and handshake regression tests protect validation ordering.
