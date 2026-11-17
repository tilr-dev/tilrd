# Proposal: The core's server, files, exec, clipboard, host and agent methods on the bus

Зависит от: bus-core-panes

Стартует после: bus-broker-core (contract); мержится после: bus-core-panes

Владеет: `crates/tilrd/src/bus/core/{server,fs,exec,clipboard,host,agent}.rs`,
`tests/common/bus_services.rs`, its block of `crates/tilrd/tests/bus_terminal.rs`, the
supervisor hooks named in the design. Merges after `bus-core-panes` and rebases on it (shared
`supervisor.rs` and `bus_terminal.rs`); it may be written in parallel.

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` T2, T3, T6,
B14, O3): every `tilr.v1` capability is kept on the bus. This change moves the rest of T2 after
panes: `server.*`, `fs.*`, `exec.*`, `clipboard.*` and the methods clients serve and tilrd calls
(`host.open`, `ssh_agent.*`).

## What Changes

- `server.info|reload|update|command` (`server.command` for the owner on the Unix socket only).
- `fs.*` calls, `fs.read` stream (bulk, chunks ≤ 256 KiB), chunked `fs.write`, watches as events
  `fs.changed.<watch_id>`.
- `exec.run` stream (bulk), `exec.input` (`{taken}`, 4 MiB unacknowledged kills), `exec.signal`.
- `clipboard.put` (chunked), `clipboard.get` stream, event `pane.copied` with scope `pane:<id>`.
- tilrd calls `host.open` on the pane's `pane.output` consumers that provide it, and
  `ssh_agent.request|close` on the one that opened its stream last (T6).
- A feature turned off (`--no-files`, `--no-exec`, ...) means its methods are not provided
  (B14); `server.info.features` lists `clipboard`, `files`, `host-events`, `agent`, `exec`
  (`sandbox` comes with `sandbox-linux`).

## Capabilities

### New Capabilities
- `bus-terminal`: `server.info`, files, file chunks, exec, exec input and server commands,
  copies and the shared clipboard, host events, the SSH agent, reload and update by call.

## Non-goals

- Moving the tests of today's `files`, `exec`, `clipboard`, `agent`, `update` files to the bus
  (`bus-test-clients`); `tilr edit --wait` (`files-and-edit-followups`, deferred until after this
  change, O6).

## Impact

`bus/core/*.rs` of the namespaces above, hooks in `supervisor.rs`, `files.rs`, `exec.rs`,
`clipboard.rs`, `host.rs`, `agent.rs`; tests. Protocol: version 1; feature flags unchanged
(only what a server can run without). Reload: `server.reload` answers before the exec; fs watches
and exec runs end with the caller's connection, as today. Nothing released from `main` (O5).
