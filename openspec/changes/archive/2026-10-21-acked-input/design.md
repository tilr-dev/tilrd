## Delivery contract

`ClientMessage.input_seq` counts from one independently for each pane within a stable `Hello.client_id`. It is separate from the connection-specific latency `seq`. Every terminal input (key, mouse, paste, raw, scroll and resize) is queued before sending. The authenticated identity includes the transport trust domain and access token; clients cannot supply another identity to a pane.

`InputAck` names a pane and its cumulative applied sequence. A pane acknowledges byte-producing input only after the last encoded byte is written to the PTY. Empty encodings and state changes still receive acknowledgements. An accepted sequence is recorded before encoding: replay of partially written input cannot append its bytes again. Completed duplicates reissue the acknowledgement. The ordered pending boundaries, accepted counters and applied counters survive exec with the pending bytes. The pane handover format changes in place under `STATE_FORMAT = 3`; `PROTOCOL_VERSION` remains 3. Both peers implement this delivery contract without a compatibility flag.

During Hello the supervisor queries each live pane in its existing input pipe. HelloAck includes that identity's applied counters after these ordered queries complete. Clients remove acknowledged entries and replay the remainder in order, retaining their delivery counters across connections. Screen frames and subscriptions do not acknowledge delivery. Anonymous, unnumbered diagnostic requests retain their existing behavior; bundled interactive clients use stable identities and numbered input.

## Client memory and overflow

The shared outbox allows at most 4 MiB of encoded frames and 4096 outstanding frames per client. Admission fails before advancing the counter or sending any part of a rejected frame. The terminal displays the overflow error and continues retaining admitted input; the driver returns an explicit error. Acknowledgement frees capacity. Explicit detach or client process exit ends the in-memory session; server process crashes and persistence across client restarts are outside exec/reconnection recovery.

## Typing through an update

The old typing case retried only writes which returned an error. A successful socket write does not prove that the supervisor read it before exec; this explains the missing numbered line. The driver now owns the replay queue, so that case sends each logical line once and checks the entire resulting file without an allowance for lost lines.

## Verification

Pause failpoints coordinate connection loss before pane application, loss after application before acknowledgement, and reload with pending byte boundaries. Tests observe files or acknowledgements as barriers rather than sleeping. Outbox tests exercise cumulative acknowledgements, per-pane numbering, replay and overflow. The existing end-to-end update case checks no loss or duplication.
