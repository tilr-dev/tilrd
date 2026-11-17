# Proposal: Tilr Web pages as bus participants

Зависит от: bus-streams-events, bus-pane-rights

Стартует после: bus-broker-core (contract); мержится после: bus-streams-events, bus-pane-rights

Владеет: the Origin check on the bus handshake in `crates/tilrd/src/bus/transport.rs` (after
`bus-broker-core`), origin rules in `bus/acl.rs` (after `bus-rights`), `tests/common/bus_web.rs`,
`crates/tilrd/tests/bus_web.rs`, README "The bus" Tilr Web subsection.

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` Part W, O3):
the Tilr app's window and each JS plugin in it are full participants over WebSocket with a
Biscuit token: they call, subscribe, stream and provide methods.

## What Changes

- The existing Origin check applies to the bus handshake (W2); a token minted with `origin`
  authorizes only on a connection with that `Origin` (the fact itself comes from `bus-rights`).
- Providers from pages with `LAST_ACTIVE` and `Activate` (B11, W4); a closed window makes its
  methods `unavailable`.
- A page's identity is its token's name across windows and reconnects; it renews its token with
  `bus.renew` without reconnecting (W3).
- README: how the app mints plugin tokens (W3).

## Capabilities

### New Capabilities
- `bus-web`: a page is a participant; origins on the handshake; tokens bound to an origin;
  gone providers; two windows, one provider; identity and renewal.

## Non-goals

- Changes in Tilr.app (another repository; it moves after this phase, M2); the HTTP gateway
  (deferred).

## Impact

`bus/transport.rs`, `bus/acl.rs`, tests, README. Protocol: version 1, no flag. Reload: page
connections end and reconnect as every WebSocket participant's; finding its panes after a
reload is `bus-reload`'s scenario "An outside participant reconnects after a reload" (moved
there from this change). Nothing released from `main` (O5).
