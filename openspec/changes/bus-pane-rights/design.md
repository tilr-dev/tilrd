# Design: Pane rights on the bus

Shared design: `openspec/design/unified-bus.md` §T7 (pane rights), §T8 (panes outlive their
creator), §R3 (pane facts), §R6, §R9. This change's part follows.

## Decisions

- **Through the contract.** The core calls `Authorizer::authorize` with `AuthzRequest::
  PaneCreate{profile, overrides}` before creating a pane and `PaneAccess{pane_id, creator}` for
  every pane-scoped method and event; the stub of `bus-broker-core` allowed both, the Biscuit
  authorizer of `bus-rights` gets the T7 rules here. `created_by` is already stamped
  (`bus-core-panes`).
- **The owner** holds every pane right (`access:any`), as today's ambient access.

## Reload and reconnect

The creator's next connection (same identity) accesses its panes again (T8); across a reload
with `bus-reload`'s `created_by` handover.

## Backpressure

Pane decisions use the decision cache of R7 (key includes the scope `pane:<id>`), so event
filtering costs a cache lookup per subscriber and event.
