## Why

The shared-buffer clipboard test sends Subscribe on a new connection and immediately
copies through a different connection. Sending Subscribe does not establish that the
server has registered the viewer, so the copy can reach no clipboard-capable viewer.

## What Changes

- Wait for the later client's initial full frame before triggering the next copy.
- Hold viewer registration with the existing subscription pause hook to verify that
  the test cannot start copying before the subscription is ready.

## Capabilities

No requirement changes. The existing clipboard-host-events requirement "A copy nobody
takes stays in the shared buffer only" already defines this behavior.

## Impact

Test code only; no protocol, feature flag, runtime state, reload or update changes.

## Non-goals

- Change clipboard delivery or replay old copies.
- Change the generic send-only subscription helper used by reconnect tests.
