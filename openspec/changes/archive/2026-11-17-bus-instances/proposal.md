# Proposal: Bus instances from bus.toml

Зависит от: bus-broker-core

Стартует после: bus-broker-core; мержится после: bus-broker-core

Владеет: `crates/tilrd/src/bus/{runner,config,control}.rs`, `tests/common/bus_instances.rs`, its
block of `crates/tilrd/tests/bus.rs`, README "The bus" instances subsection (`bus.toml`).

## Why

Owner's decisions of 2026-11-17 (`openspec/design/unified-bus.md` B10, B13,
O3): components (Deno, Python, any program speaking the bus) run as instances tilrd starts and
supervises from its own `bus.toml`; an instance that loses its connection is restarted.

## What Changes

- `bus.toml` (B10): strict TOML, read at start and on `bus.reload_config` (difference applied);
  a bad file at start leaves the server serving panes with no instances; the first round's
  `calls` key refused naming `rights`. `[instances.<n>.rights]` is parsed into the `Rights` of
  `bus-broker-core`'s contract (enforced by its stub authorizer until `bus-rights`).
- Runner: spawn in its own session, fd 3 (`TILR_BUS_FD`) or instance socket (`TILR_BUS_SOCK`,
  `TILR_BUS_TOKEN`, consumed once), environment, output to the log as `bus <name>: <line>`, Hello
  within 30 s; supervision with backoff (200 ms doubling to 5 s, reset after 10 s up).
- Control methods `bus.instances`, `bus.methods`, `bus.start`, `bus.stop`, `bus.reload`,
  `bus.reload_config`, serialized in one queue.
- Instance transports (fd, instance socket) on the broker (`BrokerHandle::attach_stream`).

## Capabilities

### New Capabilities
- `component-bus`: one protocol on every transport, the 16 MiB limit, `bus.toml`, bad
  configuration, spawning, the instance socket, restarts, stop/start/reload, config reload,
  listing, instance output.

## Non-goals

- Instance tokens (`bus-rights`: minted at each start from `rights`); handing instances over a
  reload (`bus-reload`); `tilr bus` (`bus-tilr-client`); sandboxing instances (shared design).

## Impact

`bus/{runner,config,control}.rs`, tests, README, `.config/nextest.toml` (slow tests). New
config file `bus.toml` (optional; no file = no instances). Protocol: version 1, no flag. Reload:
until `bus-reload`, a reload stops every instance before the exec (SIGTERM to its group, as
`bus.stop`) and the successor starts them again from `bus.toml`; instances do not survive a
reload yet. Nothing released from `main` (O5).
