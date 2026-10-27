## Cause and ordering

The test client's Subscribe helper waits only for the WebSocket send. The command
that produces the next clipboard copy travels over the original client's connection.
Those connections have no shared ordering guarantee. The supervisor delivers a copy
only to currently registered viewers with clipboard-write; the shared store is not a
replay queue. Command status 0 therefore does not prove that the later client got it.

## Readiness barrier

Wait for the initial full Frame for the subscribed pane. It is produced after the
supervisor registers the viewer and forwards Subscribe to the pane process. Keep this
barrier local to the shared-buffer test: reconnect tests intentionally use send-only
Subscribe to drive the registration and full-frame pause hooks separately.

## Deterministic regression

Pause reconnect-subscribe for the pane before viewer registration. Poll the test's
subscription readiness future while the hook is held; it must remain pending. Release
the hook, await readiness, and then perform the copy and check its exact contents.
The former send-only future finishes while registration is held and fails this check.

No new production state or buffering is introduced; reload, reconnect and backpressure
behavior are unchanged.
