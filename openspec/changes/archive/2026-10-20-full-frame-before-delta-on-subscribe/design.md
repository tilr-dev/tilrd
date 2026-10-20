# Cause and fix

Subscribe inserts the client into pane.subscribers before the pane process handles the request. on_pane_output currently sends any in-flight delta to that new subscriber. The pane only sets want_full when it handles Subscribe, so registration does not imply the next output is full.

Register the client in its existing per-pane lagging map on every Subscribe, preserving any own_seq already recorded. This map already suppresses delta, carries the latest own input acknowledgement into the full frame, and resumes ordinary delivery on that frame. Unsubscribe removes the entry. Other subscribers continue receiving delta; backpressure still uses the same bounded outbox and full-frame recovery.

The regression drives the real supervisor handlers with a fake pane channel: Subscribe is captured before injecting old delta, another client’s delta, full, and subsequent delta. It checks new and existing subscribers separately and verifies own input acknowledgement recovery. No scheduler timing is needed.

## Regression evidence

On 2026-10-20, `cargo test -p tilrd --bin tilrd a_subscriber_gets_full_before_delta --features tilrd/failpoints --locked` failed against the unchanged routing code with `delta arrived before full` (task log `subscribe-red.log`). With the Subscribe recovery entry it passes (`subscribe-green.log`). `cargo test -p tilrd --test metrics a_client_that_reconnects_tells_its_input_from_before --features tilrd/failpoints --locked` also passes (`metrics-reconnect.log`). The fake channel establishes the ordering directly, rather than inferring it from an intermittent metrics failure.
