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
- **Broker files untouched** *(coordinator's decision after the review of the broker)*: this
  change does not edit `bus/broker.rs` or `bus/transport.rs` (after `bus-broker-core` only
  `bus-streams-events`, then `bus-reload`, do); it uses the broker's API: `BrokerHandle::attach_stream`
  for an instance's connection, `ConnInfo.provides` for the listing, `drain_connection` to stop
  an instance, `ConnState::Ended{code, message}` to see a lost connection.

## Reload and reconnect

Interim reload above. An instance's own restart makes a new process and connection; it
resubscribes and keeps its name (identity `instance:<name>`).

## Backpressure

Instance connections use the same lanes and credits as WebSocket ones (B7, B8); the runner's log
reader is line-bounded (a line over 64 KiB is cut) *(added by the split)*.

## Deviations in the implementation

What the code froze where it differs from or adds to the text above; the code is the contract.

- **Shape.** `Runner` (`bus/runner.rs`) is a handle on a table of instances; each instance that is
  meant to run has one lifecycle task that starts the process, waits for its `Hello`, watches
  process and connection, and starts it again after the pause. The operations that change
  something (`bus.start`, `bus.stop`, `bus.reload`, `bus.reload_config`) take their turn in one
  queue (a fair mutex, run on a task of their own, so a caller that goes away does not stop one
  halfway); `bus.instances` and `bus.methods` read the table at any time, a reload in progress
  included (the "draining is not offered" scenario needs that).
- **Started from `control::register`.** The runner is made and its first instances started when the
  core's registry is built, and its handle is put in a `CoreState` slot; no start-up edit of
  `supervisor.rs` was needed. Three one-line hooks are, because a reload must stop the instances
  before the exec and a shutdown must not leave them: `bus::runner::stop_for_exit` at shutdown and
  before the exec of a reload, `bus::runner::resume_after_failed_exit` when the exec failed and the
  server carries on.
- **The interim reload stops without draining.** `stop_for_exit` sends SIGTERM to every group at
  once (SIGKILL after 3 s), for the supervisor does not serve while it waits; a planned `bus.stop`
  drains. A drain in progress (`bus.stop`, `bus.reload`, `bus.reload_config`) ends at once when
  the server exits or execs: the calls still in flight are cancelled and the instance stopped. The
  reload stops the tunnel and the instances at the same time.
- **A drain is at most 30 s.** `drain_ms` over 30000 is a `bus.toml` error (`bad_config` on
  `bus.reload_config`), and `bus.stop` with it is `bad_request`.
- **The group is killed before its leader is reaped.** The runner waits for the leader's exit with
  `WNOWAIT`, sends SIGKILL to the group while the zombie still holds its id, then reaps it; a
  reused pid is never signalled.
- **Descriptors in the child.** Between fork and exec the child closes the server's descriptors it
  listed before the fork only if they lack `FD_CLOEXEC` (the others go at the exec): a listed
  number may be another descriptor by then.
- **Control rights are checked in the methods.** The broker checks `Call{bus.reload}` against the
  caller's `call` right; the method then checks `Control{action, instance}` itself (actions
  `list`, `start`, `stop`, `reload`, `reload_config`). Until `bus-rights` an instance needs both
  in its `rights`. `bus.reload` of a module or `all` takes `Control{reload}` without an instance
  (the right over every instance) before the names are looked up, so a caller without it is
  `denied` alike for a module that exists and one that does not.
- **The log says what the listing shows.** Each change of an instance's state (`started`, `is
  up`, `is draining`, `starts again in`, `is stopped`) is logged after it is made, so a reader of
  the log finds it in `bus.instances`; the tests wait on these lines instead of polling.
- **An instance starts with no signal blocked.** The supervisor's threads hold SIGTERM, SIGINT,
  SIGHUP and SIGUSR1 for the reload; an instance is spawned through `spawn_unblocked`, like a
  pane, or it would inherit them blocked and only end by SIGKILL after 3 s.
- **A caller that acts on itself** gets its reply at once, with `new_pid` 0 for itself, and the
  stop or reload follows after 100 ms, so the reply is on the wire first.
- **Starting an instance that runs** answers its pid; reloading one that is stopped is
  `bad_request` (start it first); a failed start keeps its instance supervised, like a crash.
- **A refused `Hello`** (`reserved`, a denial) ends the attempt at once and is restarted with the
  pause, for the runner sees the end of the connection and does not wait for the 30 s.
- **Names.** An instance name is its identity and the name of its socket file: letters, digits, `.`,
  `_`, `-`, starting with a letter or a digit.
- **Configuration directory.** The directory of `server.toml` as the environment names it
  (`TILR_CONFIG_DIR`, `XDG_CONFIG_HOME`, `~/.config/tilr`); `--config` is not followed. When the
  environment names none, there are no instances (never `bus.toml` of the working directory).
- **A `bus.reload_config` that fails part way** answers the first error; `bus.toml` is taken all
  the same, and the message says what was applied (stopped, reloaded, started, kept) and which
  instances are not up.
- **Test builds shorten the Hello limits** with `TILR_TEST_BUS_HELLO_MS` (a process, 30 s) and
  `TILR_TEST_BUS_SOCKET_HELLO_MS` (a connection on an instance socket, 10 s), read only with the
  `failpoints` feature.
- **`call_timeout_ms`** is read and kept (`Runner::call_timeout`) and applied once the broker
  offers the setter (`BrokerHandle::set_call_timeout`); until then a call without a deadline gets
  the broker's own default.
- **Listings.** A participant's `address` is the transport (`unix`, `tcp`); the broker does not
  keep a peer address for it.
