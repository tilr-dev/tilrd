# Design: The core's server, files, exec, clipboard, host and agent methods

Shared design: `openspec/design/unified-bus.md` §T2 (rows of these namespaces), §T3, §T6
(copies, host events, SSH agent), §B14 (features), §B7 (bulk lane). This change's part follows.

## Decisions

- **Same paths as `tilr.v1`.** Each method reaches the existing code of `files.rs`, `exec.rs`,
  `clipboard.rs`, `host.rs`, `agent.rs` through the supervisor handle; hooks *(added by the
  split)*: the clipboard-write push also emits `pane.copied`; the host-events and agent sockets
  pick a bus consumer of the pane (one that provides `host.open` / `ssh_agent.request`) when no
  `tilr.v1` client is chosen, by the rule of T6.
- **A bus smoke test per namespace** *(added by the split)*: the requirements name today's
  `files::*`, `exec::*`, `clipboard::*`, `agent::*` tests, which speak `tilr.v1` until
  `bus-test-clients`; meanwhile `bus_terminal::files_exec_and_the_clipboard_answer_on_the_bus`
  and `bus_terminal::host_open_and_the_agent_are_called_on_a_watcher` check the bus side.

## Reload and reconnect

`server.reload` replies, then reloads as SIGUSR1 does; `server.update` streams `tilrd update`'s
output and ends with its exit (the reload follows, as today). Watches, exec runs and clipboard
transfers end with the caller's connection.

## Backpressure

`fs.read`, `exec.run`, `clipboard.get` and `server.update` are bulk streams paced by the
reader's credit (B7); uploads (`fs.write`, `clipboard.put`) are paced by their replies;
`exec.input` kills at 4 MiB unacknowledged (T3).
