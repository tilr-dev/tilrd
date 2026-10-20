# Cause and evidence

`FsWrite` runs in its own task. Its receiver durably renames the temporary file,
removes recovery metadata, and only then returns the result that queues `FsAck`.
The independent directory watcher debounces native events and queues `FsChanged`.
Both messages enter the same supervisor queue; the files protocol does not promise
that the acknowledgement precedes the notification.

The watched-directory test previously called `write()` (which waits for `FsAck`)
and then `changed()`. `answer()` uses `Client::wait_for`, which discards messages
that its predicate does not select. A notification arriving before the ack was
therefore consumed and lost. The later `changed()` waited for an event already
received, eventually hitting the existing 20-second client deadline.

The original TILR-151 and TILR-85 logs contain no per-message evidence, so their
exact historical message order cannot be recovered. The controlled reproduction
below establishes a concrete defect in that same wait sequence, with the same
timeout, without attributing unrecorded events to those historical runs.

## Controlled reproduction (2026-10-20)

`a_watched_save_can_be_reported_before_its_ack` starts the real server with
`files-write-before-ack=pause`. The writer pauses after the durable rename on a
blocking thread, leaving the watcher and supervisor free. The test waits for an
event-driven log notification emitted **after** `FsChanged` is queued, then
releases the writer. It does not read the client connection before exercising
the same result helper as the original watched-directory test.

With the original helper semantics (`ack(client, request_id).await` followed by
`changed(client, path).await`), the regression fails. Evidence from
`logs/watch-order-before-unconfined.log` (outside git), in UTC:

| Time | Event |
| --- | --- |
| 15:47:29.947623 | Native rename notification names `watched/old.txt` |
| 15:47:30.049419 | Watcher queues MODIFY |
| 15:47:30.065451 | Client waiting for ack reads and discards `FsChanged` |
| 15:47:30.066075 | Client receives successful `FsAck`, request 2 |
| 15:47:30.066473 | Client begins waiting for the lost change |
| about 20 seconds later | `Client::next` panics: `timed out waiting for the server` |

After the fix, `logs/watch-order-after.log` shows notification at
15:48:45.492091 and acknowledgement at 15:48:45.493059; the regression passes.
The result helper retains the notification and waits until both results arrive.
This is local to watched-save assertions; `Client::wait_for` retains its existing
semantics for all other integration tests. Request errors are still asserted.

Recheck using:

```sh
cargo test -p tilrd --test files a_watched_save_can_be_reported_before_its_ack --features tilrd/failpoints --locked -- --nocapture
```

To recheck the failure, temporarily replace `written_and_changed` with the
original ack-then-change sequence above and run the same command. Restore the
helper afterward. No timeout change, retry or synchronization sleep is involved.

## Environment observation

On 2026-10-20, the same command inside the Codex macOS sandbox received no native
FSEvents and stopped at the watcher-queued barrier. Temporary native callback
diagnostics also recorded no callbacks (`logs/watch-order-diagnostics.log`).
Running the command outside that sandbox delivered native events and reached the
intended regression failure. Recheck with the command above in both execution
contexts; run watcher checks outside the sandbox while this difference persists.
No repository SDK, cache or watcher configuration workaround was introduced.

The existing broad watched-directory test retains its pre-existing post-unwatch
delay; the new regression and the corrected save wait use event-driven ordering.
