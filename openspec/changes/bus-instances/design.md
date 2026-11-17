# Design: Bus instances from bus.toml

Shared design: `openspec/design/unified-bus.md` §B10 (config, spawn, supervision, control,
reserved names), §B2 (identities), §B3 (stream-socket framing). This change's part follows.

## Decisions

- **Rights before Biscuit.** `[instances.<n>.rights]` is parsed into `Rights` and handed to the
  broker with the instance's `Principal`; the stub authorizer of `bus-broker-core` enforces it
  as plain patterns. `bus-rights` replaces the enforcement with tokens minted from the same
  `Rights`; nothing in `bus.toml` changes.
- **Interim reload** *(added by the split)*: before the exec of a reload, the runner stops every
  instance (SIGTERM to its group, SIGKILL after 3 s) and the successor starts the `autostart`
  ones again. `bus-reload` replaces this with the handover of B13.
- **Scenarios that name `tilr bus`** are run through the matching `bus.*` call until
  `bus-tilr-client` adds the command; scenarios that name `tilrd`'s methods (the listing's
  `pane.output` line) are added to `bus::participants_and_methods_are_listed` by
  `bus-core-panes`.
- **Instance variants of the broker's tests.** `bus::reserved_namespaces_cannot_be_provided`
  gets the instance case (restart with backoff after `reserved`) here.

## Reload and reconnect

Interim reload above. An instance's own restart makes a new process and connection; it
resubscribes and keeps its name (identity `instance:<name>`).

## Backpressure

Instance connections use the same lanes and credits as WebSocket ones (B7, B8); the runner's log
reader is line-bounded (a line over 64 KiB is cut) *(added by the split)*.
