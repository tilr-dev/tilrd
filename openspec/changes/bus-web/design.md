# Design: Tilr Web pages as bus participants

Shared design: `openspec/design/unified-bus.md` Part W (W1-W4), §B11 (`LAST_ACTIVE`), §R6
(`origin` fact). This change's part follows.

## Decisions

- **The existing Origin check**, applied in `bus/transport.rs` to the bus handshake as it is to
  `tilr.v1` today (`--allow-origin`, `allow_origins`, the built-in origins).
- **Origin as a fact** is minted and supplied by `bus-rights`; this change checks it end to end
  with page-like clients (a WebSocket client sending `Origin`).

## Reload and reconnect

Page connections end on a reload and reconnect with their token (renewed with `bus.renew` when
due). That their panes are found again after a reload is checked by `bus-reload` (scenario "An
outside participant reconnects after a reload" of `reload`); this change checks reconnecting
without a reload.

## Backpressure

As every WebSocket participant (B7, B8); `pane.output` latest-wins for pages that lag (T4).
