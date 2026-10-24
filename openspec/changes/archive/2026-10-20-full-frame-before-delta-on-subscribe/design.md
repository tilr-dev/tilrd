# Cause and fix

Subscribe inserts the client into pane.subscribers before the pane process handles the request. on_pane_output currently sends any in-flight delta to that new subscriber. The pane only sets want_full when it handles Subscribe, so registration does not imply the next output is full.

Register the client in its existing per-pane lagging map on every Subscribe, preserving any own_seq already recorded. This map already suppresses delta, carries the latest own input acknowledgement into the full frame, and resumes ordinary delivery on that frame. Unsubscribe removes the entry. Other subscribers continue receiving delta; backpressure still uses the same bounded outbox and full-frame recovery.

The entry made by Subscribe is marked `subscribed`: waiting for the first full frame is not lag, so the delta skipped meanwhile is not counted in `tilrd_pane_frames_skipped_total` (metrics spec: one per frame and lagging client). If the client's queue lags while it waits, the mark is dropped and the skips count as for any lagging client. An entry a lagging client already has stays unmarked. Forwarding Subscribe sets `pane.full_asked`, since the pane answers it with a full frame; a delta skipped before that frame does not ask the pane for a second one.

The regression drives the real supervisor handlers with a fake pane channel: Subscribe is captured before injecting old delta, another client’s delta, full, and subsequent delta. It checks new and existing subscribers separately and verifies own input acknowledgement recovery. No scheduler timing is needed.

## Regression evidence

On 2026-10-20, `cargo test -p tilrd --bin tilrd a_subscriber_gets_full_before_delta --features tilrd/failpoints --locked` failed against the unchanged routing code with `delta arrived before full` (task log `subscribe-red.log`). With the Subscribe recovery entry it passes (`subscribe-green.log`). `cargo test -p tilrd --test metrics a_client_that_reconnects_tells_its_input_from_before --features tilrd/failpoints --locked` also passes (`metrics-reconnect.log`). The fake channel establishes the ordering directly, rather than inferring it from an intermittent metrics failure.

After review (2026-10-20): the regression also asserts that the counter stays 0 and that no second Subscribe reaches the pane. Counting the skip again fails it (`left: 1, right: 0`); not setting `full_asked` fails it with `full frame asked twice`.
