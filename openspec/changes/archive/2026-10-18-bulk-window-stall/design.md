# Design

## Context

At 1163abd the writer task of each connection (`supervisor.rs:1098-1150`) sends empty liveness pings every third of the client timeout and, after bulk packets, marking pings from `BulkWindow::wrote` (every 128 KiB, and always once the window is full). The reader passes every pong payload to `BulkAcks::pong`, which ignores payloads without the `tilr-bulk:` prefix.

## Decisions

### D1. Every ping carries the position
The liveness ping's payload becomes `BulkWindow::position()` (`tilr-bulk:` + `sent` as u64 BE, 18 bytes) instead of empty. Whichever ping a client answers, its pong says it has read at least up to the bulk sent when that ping was written. Rejected: "only while the window is closed" (one more state to get right, same cost), and resending the mark on a timer while closed (adds a timer per connection; the liveness tick already exists). With `--client-timeout 0` there are no liveness pings; the full-window mark is then the only ping and nothing can supersede it, so no stall.

### D2. No lost wake-up at lag start
In `on_pane_output`'s "starts lagging" branch, after `want_drained()` the supervisor checks `client.tx.drained()` and, if the lane is already empty, sets `ask_full`. The `Tick` handler (every `LAG_TICK`, 1 s) also asks for a full frame for each pane in a client's `lagging` map when that client's lane is drained; `ask_full_frame` is idempotent per pane (`full_asked`), so this costs nothing in the common case.

### D3. Cap the acknowledged position
`BulkAcks` shares the bytes written (`sent`, an atomic the writer stores in `BulkWindow::wrote`), and `BulkAcks::pong` caps the position at it before its `fetch_max`; `BulkAcks` stays lock-free. A lying pong can then at most acknowledge everything written so far, which is the honest maximum; an honest pong is never ahead, because its ping goes out after the data it marks.

Corrected during implementation: the prepared design capped in `BulkWindow::open()` with `min(read, sent)` at the moment of the check. That does not bound anything: a pong far ahead keeps `read` above `sent` while `sent` grows, so the window stays open for ever (the unit test `a_pong_beyond_what_was_sent_opens_no_more_than_the_window` showed 100 MiB written). The cap has to apply when the pong arrives.

## Backpressure and memory

Bounds unchanged: 4 MiB queued per client, 1 MiB unread in the network. D3 restores the 1 MiB bound for a misbehaving client.

## Reload and reconnect

No state crosses a reload; a reconnect starts a fresh outbox and window, as now.
