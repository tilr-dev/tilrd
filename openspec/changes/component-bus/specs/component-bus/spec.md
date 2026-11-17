## ADDED Requirements

### Requirement: Instances come from bus.toml
The server SHALL read `bus.toml` in its configuration directory at start and on `bus.reload_config`, with only the keys `call_timeout_ms`, `drain_ms`, `[modules.<name>]` (`command`, `cwd`, `env`) and `[instances.<name>]` (`module`, `connect`, `autostart`, `calls`, `env`), and SHALL start every instance whose `autostart` is not false. A missing file SHALL mean no instances.

**Verified by**: `bus::an_instance_from_bus_toml_is_started_and_answers`.

#### Scenario: One instance
- **WHEN** `bus.toml` defines module `t` (the test runtime) and instance `t-1`, and the server starts
- **THEN** `tilr bus ls` lists `t-1` as `up` and `tilr bus call test.echo hi` prints `hi`

#### Scenario: Not autostarted
- **WHEN** an instance has `autostart = false`
- **THEN** it is listed as `stopped` and no process runs for it until `bus.start`

### Requirement: A bad bus.toml does not stop the server
A `bus.toml` with an unknown key, a wrong type, an instance naming no module, an empty `command`, a duplicate or reserved instance name (`owner`, `tilrd`, `bus…`) SHALL be logged naming the file and the reason; at start the server SHALL serve panes as usual with no instances, and `bus.reload_config` SHALL answer `bad_config` and change nothing.

**Verified by**: `bus::a_bad_bus_toml_leaves_panes_working`, `bus::reloading_a_bad_bus_toml_changes_nothing`.

#### Scenario: Misspelt key at start
- **WHEN** `bus.toml` says `[instances.t-1] modul = "t"` and the server starts
- **THEN** the log names `bus.toml` and `modul`, panes can be created, and `tilr bus ls` lists nothing

#### Scenario: Misspelt key on reload
- **WHEN** instance `t-1` runs and `bus.toml` is edited to contain an unknown key, then `tilr bus reload-config` runs
- **THEN** it exits 1 with `bad_config: ` and `t-1` keeps running with the same pid

### Requirement: A spawned instance inherits its connection
An instance with `connect = "fd"` (the default) SHALL be started in its own session with stdin `/dev/null`, its bus connection as descriptor 3, and the environment of the server plus the module's and then the instance's `env`, plus `TILR_BUS_FD=3`, `TILR_INSTANCE=<name>` and `TILR_CONFIG_DIR=<config dir>`; no other descriptor of the server SHALL be open in it.

**Verified by**: `bus::an_instance_gets_its_connection_and_names`.

#### Scenario: Environment and descriptors
- **WHEN** the test runtime reports its environment and open descriptors through `test.env`
- **THEN** it shows `TILR_BUS_FD=3`, `TILR_INSTANCE=t-1`, the instance's `env` over the module's, and descriptors 0, 1, 2 and 3 only

### Requirement: The socket transport takes one token once
An instance with `connect = "socket"` SHALL get `TILR_BUS_SOCK` (a socket `<runtime dir>/bus/<name>.sock`, mode 0600, in a 0700 directory) and `TILR_BUS_TOKEN` (32 hex characters, new for each process start); the server SHALL bind the first `Hello` carrying that token to the instance and MUST refuse any other connection to that socket, closing it.

**Verified by**: `bus::a_socket_instance_connects_once_with_its_token`.

#### Scenario: Token reused
- **WHEN** a socket instance is up and another process connects to its socket with the same token
- **THEN** that connection is closed without `HelloAck` and the instance stays `up`

### Requirement: Socket paths that are too long are a config error
A `connect = "socket"` instance whose socket path is longer than the platform's limit (104 bytes on macOS, 108 on Linux) SHALL make `bus.toml` invalid, naming the path and the limit.

**Verified by**: `bus::a_socket_path_that_is_too_long_is_a_config_error`.

#### Scenario: Deep runtime directory
- **WHEN** `TILR_RUNTIME_DIR` is a 100-byte path and an instance uses `connect = "socket"`
- **THEN** the log names that socket path and the limit, and the instance is not started

### Requirement: A connection says Hello in time
A connection to a socket instance's socket SHALL be closed if it sends no valid `Hello` within 10 seconds; a spawned instance that has not said `Hello` 30 seconds after it was started SHALL be killed and restarted with backoff.

**Verified by**: `bus::an_instance_that_never_says_hello_is_restarted`.

#### Scenario: Silent instance
- **WHEN** the test runtime is started with `TEST_NO_HELLO=1`
- **THEN** after 30 seconds its process is killed and a new one is started

### Requirement: Calls are routed by instance or by method
A `Call` with a `target` SHALL go to that instance; one without SHALL go to a live, non-draining instance that provides the method, in turn over those instances in config order. The broker SHALL set `from` to the caller's identity in every forwarded call, whatever the caller sent, and SHALL accept a `Reply` only from the connection the call was sent to.

**Verified by**: `bus::calls_go_round_robin_over_the_providers`, `bus::from_is_always_the_callers_identity`.

#### Scenario: Two providers
- **WHEN** instances `t-1` and `t-2` provide `test.whoami` and the owner calls it four times without a target
- **THEN** the replies are `t-1`, `t-2`, `t-1`, `t-2`

#### Scenario: Forged from
- **WHEN** `t-1` calls `test.from` on `t-2` with `from` set to `owner`
- **THEN** `t-2` sees `from` `t-1`

### Requirement: The sender learns why a call failed
A call SHALL be answered with `Error.code` `not_found` when its target or method is unknown, `unavailable` when the target or every provider is not up or is draining or goes away before replying, `timeout` when no reply comes within `min(Call.timeout_ms, call_timeout_ms)`, and `denied` when it is not allowed; the message SHALL name the instance or method.

**Verified by**: `bus::a_failed_call_says_why`.

#### Scenario: Unknown method
- **WHEN** the owner calls `nobody.here`
- **THEN** the reply is `not_found` naming `nobody.here`

#### Scenario: Stopped target
- **WHEN** `t-1` is stopped and the owner calls `test.echo` with target `t-1`
- **THEN** the reply is `unavailable` and says `t-1 is stopped`

#### Scenario: Slow handler
- **WHEN** the owner calls `test.sleep` for 5 s with `timeout_ms` 500
- **THEN** the reply is `timeout` after about 500 ms

### Requirement: A full queue is answered busy
The broker SHALL answer `busy` at once to a call whose callee's outbound queue is full or whose caller already has `HelloAck.max_in_flight` calls pending, and MUST NOT grow memory without bound for a slow reader.

**Verified by**: `bus::a_full_queue_is_answered_busy`.

#### Scenario: In-flight limit
- **WHEN** `t-1` sends 300 calls to a method that never replies, with `max_in_flight` 256
- **THEN** 256 stay pending and 44 are answered `busy`

### Requirement: Deadlines and cancellation reach the callee
A forwarded call SHALL carry in `timeout_ms` the time its caller has left; when a call times out or its caller's connection ends, the broker SHALL send the callee `Cancel{id}` and SHALL drop a later `Reply` to it.

**Verified by**: `bus::a_nested_call_gets_the_time_that_is_left_and_a_cancel`.

#### Scenario: Nested deadline
- **WHEN** the owner calls `test.nested` on `t-1` with `timeout_ms` 1000 and `t-1` waits 400 ms before calling `test.deadline` on `t-2`
- **THEN** `t-2` sees `timeout_ms` of at most 600

#### Scenario: Caller gone
- **WHEN** a caller's connection ends while its call to `test.sleep` runs on `t-2`
- **THEN** `t-2` receives `Cancel` with that call's id

### Requirement: Frames are limited to 16 MiB
`HelloAck.max_frame` SHALL be 16 MiB; a frame longer than that in either direction MUST close the connection as a protocol error, and a client library SHALL refuse to send a call that would exceed it with `too_large`.

**Verified by**: `bus::a_frame_over_the_limit_closes_the_connection`.

#### Scenario: Oversized frame
- **WHEN** a socket instance sends a length prefix of 16 MiB + 1
- **THEN** its connection is closed and the instance is restarted

### Requirement: A slow instance does not hold up the others
Calls between instances SHALL be routed while another instance is slow to read or to reply.

**Verified by**: `bus::a_slow_instance_does_not_hold_up_the_others`.

#### Scenario: One instance stops reading
- **WHEN** `t-3` stops reading its connection and the owner calls `test.echo` on `t-1` 1000 times
- **THEN** every call is answered within the usual time

### Requirement: Payloads are bytes the broker does not read
The broker SHALL pass `payload` and `content_type` of calls and replies unchanged, byte for byte, including empty and non-UTF-8 payloads; only the bodies of `bus.*` methods are parsed (JSON).

**Verified by**: `bus::payloads_pass_byte_for_byte`.

#### Scenario: Binary payload
- **WHEN** the owner calls `test.echo` with 1 MiB of random bytes and `content_type` `application/octet-stream`
- **THEN** the reply has the same bytes and the same `content_type`

### Requirement: Calls are denied unless allowed
A call SHALL be refused with `denied` (`<from> may not call <method>`) before routing unless the caller is `owner` or one of the caller instance's `calls` globs matches the method (`*` matches any characters); `bus.*` methods follow the same rule.

**Verified by**: `bus::calls_are_denied_unless_allowed`.

#### Scenario: Not listed
- **WHEN** `t-1` has `calls = ["test.echo"]` and calls `test.whoami` and `bus.instances`
- **THEN** both are answered `denied`

#### Scenario: Owner
- **WHEN** the owner calls `bus.instances`
- **THEN** it receives the list

### Requirement: A crashed instance is restarted with backoff
An instance whose process exits on its own, or whose connection ends while its process lives, SHALL have its process group killed and be started again after a pause of 200 ms doubling up to 5 s, reset after it has run 10 s; `bus.instances` SHALL count the restarts.

**Verified by**: `bus::a_crashed_instance_is_restarted_with_backoff`, `bus::an_instance_that_loses_its_connection_is_restarted`.

#### Scenario: Crash loop
- **WHEN** the test runtime exits at once three times
- **THEN** the starts are about 200, 400 and 800 ms apart and `restarts` is 3

#### Scenario: Connection closed
- **WHEN** the test runtime closes descriptor 3 but keeps running (`test.close_bus`)
- **THEN** its process is killed and a new process says `Hello`

### Requirement: Stopping and starting an instance
`bus.stop {instance, drain_ms?}` SHALL stop routing to it, wait for its calls up to `drain_ms`, send SIGTERM to its process group and SIGKILL after 3 s, and keep it `stopped` until `bus.start {instance}`, which SHALL start it and answer once it said `Hello`. Both SHALL be idempotent.

**Verified by**: `bus::an_instance_is_stopped_and_started`.

#### Scenario: Stop and start
- **WHEN** the owner runs `tilr bus stop t-1`, then `tilr bus start t-1`
- **THEN** after the stop no process of `t-1` runs and it is listed `stopped`; after the start it is `up` with a new pid

### Requirement: Reloading instances one at a time
`bus.reload {instance | module | all, drain_ms?}` SHALL restart the named instances one after the other, each drained first and waited for until it says `Hello`, and answer with old and new pids per instance and per-instance errors (`spawn_failed`, `start_failed` with the end of its stderr, `start_timeout`).

**Verified by**: `bus::a_module_is_reloaded_one_instance_at_a_time`.

#### Scenario: No gap for a method
- **WHEN** `t-1` and `t-2` provide `test.echo`, the owner calls it in a loop and `tilr bus reload --module t` runs
- **THEN** no call of the loop fails and both instances have new pids

#### Scenario: A caller reloads itself
- **WHEN** `t-1` calls `bus.reload {"instance": "t-1"}`
- **THEN** it receives the reply before its process is stopped

### Requirement: Reloading bus.toml applies the difference
`bus.reload_config` SHALL read `bus.toml` again and stop removed instances, start added ones, reload instances whose module or entry changed, keep others running, keep manually stopped instances stopped, and apply `call_timeout_ms` and `calls` to new calls.

**Verified by**: `bus::reloading_bus_toml_applies_the_difference`.

#### Scenario: Add, change, remove
- **WHEN** `t-2` is added, `t-1`'s `env` changes and `t-3` is removed, then `tilr bus reload-config` runs
- **THEN** `t-2` is up, `t-1` has a new pid, `t-3` no longer runs, and an unchanged `t-4` keeps its pid

### Requirement: Listing instances and methods
`bus.instances` SHALL list every configured instance with its name, module, transport, state (`down`, `starting`, `up`, `draining`, `stopped`), pid, restarts and provided methods; `bus.methods` SHALL list each method with the instances that are up and not draining and provide it.

**Verified by**: `bus::instances_and_methods_are_listed`.

#### Scenario: Draining is not offered
- **WHEN** `t-1` is being drained by `bus.reload` and `bus.methods` is called
- **THEN** `t-1` is not listed as a provider

### Requirement: The owner calls the bus through tilrd
A greeted client with full access SHALL be able to send `BusCall{request_id, target, method, payload, content_type, timeout_ms}` and SHALL receive `BusReply{request_id, payload | error, content_type}`; the call SHALL reach the instance with `from` `owner`.

**Verified by**: `bus::a_client_calls_the_bus_through_tilrd`.

#### Scenario: Over the Unix socket
- **WHEN** a client on the Unix socket sends `BusCall{request_id: 3, method: "test.from"}`
- **THEN** it receives `BusReply` with `request_id` 3 and payload `owner`

### Requirement: tilr bus call
`tilr bus call METHOD [PAYLOAD | - | --file PATH] [--target NAME] [--timeout DURATION]` SHALL send a `BusCall` and write the reply's payload to stdout unchanged and exit 0, or write `<code>: <message>` to stderr and exit 1; `tilr bus ls`, `start`, `stop`, `reload` and `reload-config` SHALL call the matching `bus.*` method and print its result.

**Verified by**: `client::tilr_bus_call_prints_the_reply`.

#### Scenario: Echo from stdin
- **WHEN** the user runs `printf 'a\0b' | tilr bus call test.echo -`
- **THEN** stdout is the 3 bytes `a`, NUL, `b` and the exit status is 0

#### Scenario: Denied or missing
- **WHEN** the user runs `tilr bus call nobody.here`
- **THEN** stderr starts with `not_found: ` and the exit status is 1

### Requirement: Instances survive a tilrd reload
On a reload the server SHALL hand its instances and their connections to the successor: the instances' pids stay the same, they stay `up` and are routable after the reload; calls in flight at the exec SHALL be answered `unavailable` and their callees sent `Cancel`.

**Verified by**: `reload::bus_instances_survive_a_reload`.

#### Scenario: Same pid after reload
- **WHEN** `t-1` is up, the server gets SIGUSR1, and the client reconnects
- **THEN** `tilr bus ls` shows `t-1` `up` with the same pid and `restarts` unchanged, and `tilr bus call test.echo x` prints `x`

### Requirement: An instance's output goes to the server log
Each line an instance writes to stdout or stderr SHALL appear in the server's log as `bus <name>: <line>`.

**Verified by**: `bus::an_instances_output_is_in_the_server_log`.

#### Scenario: A line on stderr
- **WHEN** the test runtime writes `hello log` to stderr
- **THEN** the server log has a line ending with `bus t-1: hello log`
