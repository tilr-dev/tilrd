# Design: The bus across tilrd's reload

Shared design: `openspec/design/unified-bus.md` §B13 (tilrd's reload), "Reload and reconnect",
§M3 (shells across the switch), §T7-T8 (`created_by`), §P4 (snapshot), §S5/§S10 (the sandbox
stays in the kernel). This change's part follows.

## Decisions

- **One owner of reload state.** Everything that crosses a reload in this phase is written and
  read here: `bus-state 1`, and `supervisor-state 3` with the profile snapshot, `created_by` and
  `sandboxed`. The changes that made those fields (`pane-profiles`, `bus-core-panes`,
  `sandbox-linux`) did not change the state formats (O4).
- **Internal schemas out of `tilr.proto` now.** Adding fields to `PaneState` would otherwise edit
  `tilr.proto`, which no bus change touches (O2, O4); moving the internal messages to
  `internal.proto` here leaves `tilr.proto` with the client messages only, and `remove-tilr-v1`
  deletes it whole.
- **Shells across the switch (owner, 2026-11-17; was the open question of `unified-bus`).**
  Moving from a build before the bus to a bus build is a restart of tilrd, not a reload: the
  bus build reads `supervisor-state 3` only, so the old server's `state-formats` check refuses
  the reload, logs why, and keeps serving its panes; the update asks for a restart, and open
  shells end at it. Reloads between bus builds work as before. The test of this is
  `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`, updated with the
  case "from a `supervisor-state 2` build": a clear refusal with its log line, panes and clients
  kept. The release notes of the new minor say so (`remove-tilr-v1`, M3).

## Reload and reconnect

B13: before the exec the broker stops routing, waits up to 1 s for calls in flight, answers the
rest `unavailable` and cancels their callees, ends open streams `unavailable`. Instances keep
their pid, connection fd (not close-on-exec across the exec), listener fd, provides, counters,
token state and subscriptions. WebSocket participants reconnect with a new `connection_id`.
Caches start empty. `tilrd state-formats`: `supervisor-state 3`, `pane-state 3`, `bus-state 1`,
`unix-socket 1`, `reload-check 1`.

## Backpressure

Frames an instance writes during the exec wait in its socket buffer; nothing is queued in
memory across the exec.
