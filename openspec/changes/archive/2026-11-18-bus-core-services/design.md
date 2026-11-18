# Design: The core's services on the bus

Shared design: `openspec/design/unified-bus.md` §T1 (the core as a native participant), §T2
(mapping), §T6 (files, exec, clipboard, host events, agent), §B9 (JSON). This change's part
follows.

## Decisions

- **Entry points without the protocol.** Each service gets an API in its own module that
  takes and gives neutral types, and both protocols sit on it:
  - `files::Access` (`Files::access()`): `list`, `stat`, `mkdir`, `rename`, `delete`
    (blocking, run in `spawn_blocking`), `watch(path, named, ChangeSink)`, `open_read` giving
    a `Reader` of chunks, `begin_write` giving an `Upload` (`take`, `finish`; dropped
    unfinished, its temp file goes).
  - `exec::Execs::start(client, client_id, Program, Arc<dyn Report>)`, `input(.., ack)`,
    `signal`, `cancel`: a `Report` says where `started`, output and the exit go (`tilr.v1`:
    `ClientReport` to the client's queue; the bus: `BusReport` to the stream).
  - `clipboard::Receiver::put(client, Put) -> Taken`.
  The `tilr.v1` handlers keep every message, log line and failpoint they had.
- **Through the handle, in order.** A call is decoded and queued on the supervisor before its
  handler returns its future (`server::unary`), so the chunks of one `fs.write` or
  `clipboard.put` transfer, and `exec.input` calls, are taken in the order the caller sent
  them. Blocking file work runs off the supervisor.
- **Connection ids are client ids.** A bus connection id and a `tilr.v1` client id come from
  one counter (`bus-broker-core`), so a connection is the "client" of `Execs`, the files'
  uploads and the clipboard `Receiver`. What a connection made (watches, uploads, programs,
  an upload of the clipboard) goes when it ends (`broker.watch_connection`, one watcher per
  connection and service, `server::on_connection_end`). A call whose connection has ended by
  the time the supervisor takes it (`on_connection_end` answers `false`) keeps nothing and is
  refused: a first `fs.write` chunk starts no upload, `clipboard.put` keeps no transfer,
  `exec.run` starts no program, `fs.watch` keeps no watch.
- **Limits per connection.** At most `fs::MAX_WATCHES_PER_CONNECTION` (32) watches, each a
  watcher of the system's (an inotify instance on Linux, whose default limit is 128 per
  user); `fs::MAX_UPLOADS_PER_CONNECTION` (16) uploads in progress, each with a temporary file
  open; `exec::MAX_EXECS_PER_CLIENT` (64) programs, as on `tilr.v1`. One more is `busy`, the
  code the broker gives a caller over its share of calls in flight.
- **Streams split to the window.** `fs.read`, `exec.run` output and `clipboard.get` cut their
  data so that no chunk passes the stream's window (`common::room`, `common::split`), besides the
  256 KiB of `tilr.v1`. A cancelled `exec.run` stream cancels the program (its group is
  killed), as `ExecCancel` does; so does a stream whose reader is gone. The cancel guard is
  made before the start is queued, and its cancel is queued behind the start, so a handler
  aborted at any point after it queued the start takes the program with it.
- **`server.update` and `server.command` are programs.** Both run the server's own executable
  (`tilrd update`; `tilrd <argv>`) through `exec.run`'s path; no protocol-free entry point is
  needed in `update.rs`.
- **Who may.** `server.reload` and `server.update` ask the authorizer for the `control` right
  `reload` (the owner has it); `server.command` is for the owner on the Unix socket only
  (`denied` on TCP, even with the access token), as `tilr.v1`'s `ServerCommand`.
- **Errors.** A refusal a user should read (outside the roots, exists, too large, read-only, a
  disk error, stdin closed or overrun) is `bad_request` with the message of `tilr.v1`; a
  program that did not start `spawn_failed`; an `exec_id`, watch or copy that is not the
  caller's or not there `not_found` (`exec.input` and `exec.signal` alike); a limit per
  connection `busy`. `exec::Execs` tells these apart with `exec::Refusal`.
- **Uploads clean up before they answer.** A failed or abandoned upload removes its
  temporary file and its note of the write before the failure is answered
  (`files::Upload::abandon`, and `finish` on failure), as `tilr.v1`'s `receive` always did; a
  drop without either still removes them, in the background.
- **`pane.copied`.** The hook in `Supervisor::send_copy` (`bus::core::clipboard::copied`)
  keeps the latest copy and emits `pane.copied` with scope `pane:<id>` before the `tilr.v1`
  `ClipboardWrite`s; `clipboard.get` streams it while it is the latest, to a caller the
  authorizer lets access the copy's pane (`AuthzRequest::PaneAccess`, as `pane.*`); to one it
  does not, it answers `not_found`, as for no copy.
- **Test-only detours.** `fail::armed(name)` tells whether a test set an action for a
  failpoint without passing it: the supervisor's task cannot block on a pause, so a chunk of
  `fs.write` is held on a task of its own (`bus-fs-write-late`) only when a test asks.
  `bus-exec-started` holds `exec.run` between the start and the handler taking it up.
- **Broker files untouched**: this change uses only the broker's API (`BrokerHandle::emit`,
  `watch_connection`, `authorizer`, `principal_of`).

## Gaps

- **`host.open` and `ssh_agent.*` are not done here.** T6 says tilrd calls `host.open` on each
  `pane.output` consumer of the pane that provides it, and `ssh_agent.request` on the one that
  opened its stream last. `BrokerHandle::call` resolves `CallOptions.target` as an identity
  (every owner connection is `owner`), so tilrd cannot call one connection. They are done by
  a separate change (a per-connection call in the broker plus `host.open` and `ssh_agent.*`);
  until then both stay on `tilr.v1`.
- **The reload reply is not flushed.** `server.reload` answers, then sends the reload to the
  supervisor; nothing waits for the bus connections' queues to be written before the exec, so
  the reply may be lost with the connection: the requirement says the caller sees the reply
  or the end of its connection, and the test takes either. `bus-reload` drains the bus at a
  reload and makes the reply guaranteed.
- **`fs.changed.<watch_id>` is not scoped to its watcher.** `BrokerHandle::emit` takes a
  scope `pane:<id>` only and delivers to every subscriber whose rights allow the subject; it
  has no scope of one connection, and adding one is the broker's (`broker.rs`, `authz.rs`).
  So any connection that may subscribe to `fs.changed.*` sees the changes of every watch.
- **`pane.read` waits at most about 9.9 s**, not 25 s: T9's cap is under the broker's 10 s call
  deadline (B5). README says so; the design's 25 s stands for a later longer deadline.
- **Requirement "The terminal is served on the bus only"** of the former single change holds
  a clause (`tilr.proto` absent, a CI step) that is `remove-tilr-v1`'s; this change adds the
  listing part as its own requirement, without `pane.profiles` (`pane-profiles`).
