# Proposal: Component bus in tilrd

Зависит от: - (its `tilr-cli` and `reload` deltas are written on top of `pane-profiles`', which comes first in the roadmap)

## Why

Decisions of 2026-10-26 (owner; Basic Memory decision log, recorded in session TILR-892 on
2026-11-17): tilrd becomes a minimal supervisor of components; configuration distinguishes a
module (what to run) from named instances; an instance's identity is a socketpair fd inherited at
spawn (`TILR_BUS_FD`, systemd-socket-activation style); the broker lives in tilrd, default deny;
payloads are opaque bytes to the broker; outside access goes through a gateway component; hot
streams may later get a direct channel granted by the broker (the protocol must allow it). On
2026-11-17 the owner decided that the bus from the prototype `runtime-tmp` moves into this repo
as workspace crates, with its specs here.

The prototype (`runtime-tmp`, `host/src/{broker,control,main,wire}.rs`, `proto/bus.proto`,
`docs/how-it-works.md`) works and lists its own limits (§10): no backpressure (unbounded
per-connection queues), one broker mutex, no direct channel, no cancellation, no dynamic method
registration, ACL without delegation, no reconnect, unchecked `sun_path` length, one reload
lock, fixed timing heuristics, no state across reload, an attach-token race, no Hello timeout,
`bus.methods` listing draining instances. Its attach instances (a client connects with a token
from a file; the host does not start it) are declared a demo deviation (§11).

## What Changes

- New crate `crates/tilr-bus`: the bus protocol (`proto/bus.proto`, package `tilr.bus.v1`),
  framing, and an async Rust client used by Rust components and tests. The broker and the runner
  (spawning, supervision) live in tilrd (`crates/tilrd/src/bus/`).
- Star topology: every frame goes through the broker; instances do not know each other's
  addresses. Call/Reply with per-connection ids remapped by the broker; `from` set by the broker;
  `trace_id`; routing by instance name or by method (round-robin over live providers).
- Protocol changes against the prototype: `Hello.protocol_version`, `HelloAck` limits,
  `Call.timeout_ms` (deadline carried down a chain), `Call.content_type`, a `Cancel` message
  (the broker tells a callee a call is abandoned), `Error.code` from a fixed documented set with
  `busy` and `too_large` added; unknown `Envelope` variants are ignored, so a later direct-channel
  grant can be added without a version bump.
- Config: own strict TOML file `bus.toml` next to `server.toml`, with `[modules.<name>]` and
  `[instances.<name>]`; reloaded live with `bus.reload_config`.
- Spawn: socketpair end as fd 3, `TILR_BUS_FD=3`, `TILR_INSTANCE`, `TILR_CONFIG_DIR`; or
  `connect = "socket"`: `<runtime dir>/bus/<name>.sock` plus a per-start token
  (`TILR_BUS_SOCK`, `TILR_BUS_TOKEN`). Supervision with backoff; start/stop/rolling reload of
  instances; config reload with a diff.
- Fixed here: bounded queues and in-flight limits (`busy`), no global broker lock, `sun_path`
  checked at config load, lost connection = instance restarted, Hello timeout, `bus.methods`
  without draining instances, timers cancelled with the reply, self-reload acted on after the
  reply is written (no 300 ms guess).
- tilrd's own reload: instances and their connections survive it (handed over like panes);
  calls in flight at the exec are answered `unavailable`.
- The owner calls the bus by hand through tilrd: `BusCall` / `BusReply` on the existing client
  protocol, and `tilr bus call|ls|start|stop|reload|reload-config`. Attach instances are not
  carried over.
- Authorization in this change: default deny with a minimal rule set — an instance may call the
  methods its `calls` globs list; tilrd's greeted full clients (the owner) may call everything.
  Change `bus-acl-biscuit` replaces it.
- Protocol (tilr.proto): `BusCall` (ClientMessage 33), `BusReply` (ServerMessage 26), additive;
  version 1 stays; no feature flag (design D12).

## Capabilities

### New Capabilities
- `component-bus`: bus protocol, broker, instances, config, supervision, control methods, the
  owner's access through tilrd, reload.

### Modified Capabilities
- `tilr-cli`: "Command set" (`bus`).
- `reload`: "state-formats names what this executable reads" (`bus-state 1`).

## Non-goals

- Biscuit tokens and real rights (change `bus-acl-biscuit`), the HTTP gateway (`bus-gateway`),
  SDKs for Deno and Python (`bus-runtime-sdks`), pane methods (`bus-panes`).
- A direct channel for hot streams (the protocol leaves room: design D3); streaming replies.
- Dynamic method registration after Hello; file watching and automatic reload on change.
- Sandboxing instances (later, on top of `sandboxed-pane`'s mechanism).
- Moving panes into a separate `tilr-panes` component (direction of 2026-10-26; not now).
- Keeping an instance's in-memory state across its own restart.
- Attach instances (a process the runner does not start).

## Impact

New `crates/tilr-bus` (workspace member; `prost`, `tokio`, already in the tree), new
`crates/tilrd/src/bus/{broker,runner,config,control}.rs`, `crates/tilrd/src/{supervisor,reload,
main}.rs`, `crates/tilr-proto/proto/tilr.proto` (`BusCall`, `BusReply`), `crates/tilr/src/`
(`tilr bus`), README ("Components" new section, "Protocol", "Reloading without closing shells"),
USE_CASES.md, `.config/nextest.toml` (slow bus tests).

Reload: new state `bus-state 1` in `server-<pid>.state` (instances, pids, connection fds,
provides, counters); `tilrd state-formats` gains the line, so a successor without the bus is not
reloaded into. Update from the previous release: no `bus.toml`, no instances; nothing changes.

Not breaking: additive client-protocol messages; the bus protocol is new.
