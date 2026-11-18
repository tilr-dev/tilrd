# Proposal: The core's services on the bus

Зависит от: bus-core-panes

Стартует после: bus-broker-core (contract: `StreamOut`, `BrokerHandle`, `SupervisorHandle`); мержится после: bus-core-panes

Владеет: `crates/tilrd/src/bus/core/{server,fs,exec,clipboard}.rs`, the protocol-free
entry points in `crates/tilrd/src/{files,exec,clipboard}.rs`, the supervisor hook in
`Supervisor::send_copy`, `tests/common/bus_services.rs`, the `bus-core-services` blocks at the
end of `tests/bus_terminal.rs`, `tests/reload.rs` and `tests/update.rs`, README "The bus"
subsection "Services" (and the two corrections in "Panes" named below).

## Why

Owner's decisions of 2026-11-17 (`openspec/design/unified-bus.md` T1, T2, T6): tilrd's core is
a native participant and the terminal moves onto the bus. `bus-core-panes` put panes on it;
this change puts the rest of what `tilr.v1` serves there: the server itself, files, programs
and the clipboard, beside `tilr.v1`, on the supervisor's existing paths.

## What Changes

- The participant `tilrd` provides `server.info|reload|update|command`, `fs.list|stat|read|
  write|mkdir|rename|delete|watch|unwatch` with the events `fs.changed.<watch_id>`,
  `exec.run|input|signal`, `clipboard.put|get`, and emits `pane.copied` with scope `pane:<id>`
  (T2, T6). `fs.read`, `exec.run`, `clipboard.get`, `server.update` and `server.command` are
  streams on the bulk lane, split to the reader's window.
- A feature turned off (`files`, `exec`, `clipboard`) provides none of its methods.
- `files.rs`, `exec.rs` and `clipboard.rs` get entry points that know no `tilr.v1` message
  (`files::Access`, `exec::Execs::start|input|signal|cancel` with a `Report`,
  `clipboard::Receiver::put`); the `tilr.v1` handlers sit on them, unchanged in behaviour.
- README "Panes": `pane.read`'s wait is bounded by the 10 s call deadline (about 9.9 s at
  most), and `pane.list` lists the running panes the caller may access.

## Capabilities

### Modified Capabilities
- `bus-terminal`: the server described, reloaded, updated and its commands run by calls;
  files, file content in chunks and watches; programs run, fed and signalled; copies and the
  shared clipboard; what a connection made, limited per connection and gone with it.

## Non-goals

- `host.open` and `ssh_agent.request|close` (T2, T6) are calls tilrd makes to the clients that
  watch a pane; the broker's API cannot call one connection, only an identity, so they stay
  on `tilr.v1` here. They are done by a separate change (a per-connection call in the broker
  plus `host.open` and `ssh_agent.*`; design.md, Gaps).
- A guaranteed `server.reload` reply: flushing the bus before the exec is `bus-reload`'s.
- Moving the `tilr.v1` tests to the bus (`bus-test-clients`), removing `tilr.v1`
  (`remove-tilr-v1`), what crosses a reload (`bus-reload`).

## Impact

`bus/core/{server,fs,exec,clipboard}.rs`, the entry points in `files.rs`, `exec.rs`,
`clipboard.rs`, one hook in `supervisor.rs`, `bus/core/panes.rs`'s pane access check opened to
`clipboard.get` (`may_access`, `Caller`), `fail::armed` in `fail.rs` (a test-only detour to a
failpoint), tests, README. Protocol: `tilr.bus.v1` version 1,
no flag; `tilr.v1` unchanged. Reload: nothing new crosses it; a watch, upload or program of a
bus connection ends with the connection, as on `tilr.v1`. Nothing released from `main` (O5).
