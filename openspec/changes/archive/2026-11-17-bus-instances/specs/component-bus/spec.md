## ADDED Requirements

### Requirement: One protocol on every transport
The server SHALL speak `tilr.bus.v1` (`crates/tilr-bus/proto/bus.proto`) on the TCP port and on the Unix socket over WebSocket with the subprotocol `tilr.bus.v1`, one `Envelope` per binary message after a flag byte (`0` plain, `1` zstd), and on instance transports (descriptor 3, instance sockets) as a u32 big-endian length followed by an `Envelope`.

**Verified by**: `bus::every_transport_speaks_the_same_envelopes`, unit test `codec::tests::small_envelopes_stay_plain` (crates/tilr-bus/src/codec.rs).

#### Scenario: Same call on three transports
- **WHEN** the test runtime runs as an fd instance, a socket instance, and a WebSocket participant (the owner on the Unix socket), and each calls `test.echo` with `x`
- **THEN** each gets `x` back

### Requirement: Envelopes are limited to 16 MiB
An `Envelope` longer than `HelloAck.max_frame` (16 MiB) after decompression, in either direction, MUST close the connection as a protocol error.

**Verified by**: `bus::a_frame_over_the_limit_closes_the_connection`, unit test `codec::tests::a_frame_over_the_limit_is_refused` (crates/tilr-bus/src/codec.rs).

#### Scenario: Oversized frame
- **WHEN** a socket instance sends a length prefix of 16 MiB + 1
- **THEN** its connection is closed and the instance is restarted

### Requirement: Instances come from bus.toml
The server SHALL read `bus.toml` in its configuration directory at start and on `bus.reload_config`, with only the keys `call_timeout_ms`, `drain_ms`, `[modules.<name>]` (`command`, `cwd`, `env`) and `[instances.<name>]` (`module`, `connect`, `autostart`, `rights`, `env`), and SHALL start every instance whose `autostart` is not false. A missing file SHALL mean no instances.

**Verified by**: `bus::an_instance_from_bus_toml_is_started_and_answers`.

#### Scenario: One instance
- **WHEN** `bus.toml` defines module `t` (the test runtime) and instance `t-1` with `rights.provide = ["test.*"]`, and the server starts
- **THEN** `tilr bus ls` lists `t-1` as `up` and `tilr bus call test.echo hi` prints `hi`

#### Scenario: Not autostarted
- **WHEN** an instance has `autostart = false`
- **THEN** it is listed as `stopped` and no process runs for it until `bus.start`

### Requirement: A bad bus.toml does not stop the server
A `bus.toml` with an unknown key (including the old `calls`, whose message SHALL name `rights`), a wrong type, an instance naming no module, an empty `command`, a duplicate or reserved instance name (`owner`, `tilrd`, `bus…`) or a socket path over the platform limit (104 bytes macOS, 108 Linux) SHALL be logged naming the file and the reason; at start the server SHALL serve panes as usual with no instances, and `bus.reload_config` SHALL answer `bad_config` and change nothing.

**Verified by**: `bus::a_bad_bus_toml_leaves_panes_working`, `bus::reloading_a_bad_bus_toml_changes_nothing`, `bus::a_socket_path_that_is_too_long_is_a_config_error`.

#### Scenario: Misspelt key at start
- **WHEN** `bus.toml` says `[instances.t-1] modul = "t"` and the server starts
- **THEN** the log names `bus.toml` and `modul`, panes can be created, and `tilr bus ls` lists nothing

#### Scenario: The old key
- **WHEN** `bus.toml` says `[instances.t-1] calls = ["test.*"]`
- **THEN** the reason says `calls` was replaced by `[instances.t-1.rights] call`

#### Scenario: Misspelt key on reload
- **WHEN** instance `t-1` runs, `bus.toml` is edited to contain an unknown key, and `tilr bus reload-config` runs
- **THEN** it exits 1 with `bad_config: ` and `t-1` keeps running with the same pid

### Requirement: A spawned instance inherits its connection
An instance with `connect = "fd"` (the default) SHALL be started in its own session with stdin `/dev/null`, its bus connection as descriptor 3, and the environment of the server (without the pane variables) plus the module's and then the instance's `env`, plus `TILR_BUS_FD=3`, `TILR_INSTANCE=<name>` and `TILR_CONFIG_DIR=<config dir>`; no other descriptor of the server SHALL be open in it.

**Verified by**: `bus::an_instance_gets_its_connection_and_names`.

#### Scenario: Environment and descriptors
- **WHEN** the test runtime reports its environment and open descriptors through `test.env`
- **THEN** it shows `TILR_BUS_FD=3`, `TILR_INSTANCE=t-1`, the instance's `env` over the module's, and descriptors 0, 1, 2 and 3 only

### Requirement: The instance socket takes one token once
An instance with `connect = "socket"` SHALL get `TILR_BUS_SOCK` (`<runtime dir>/bus/<name>.sock`, mode 0600, in a 0700 directory) and `TILR_BUS_TOKEN` (32 hex characters, new for each process start); the server SHALL bind the first `Hello` whose `instance_token` is that token to the instance and MUST refuse any other connection to that socket, closing it; a connection that sends no valid `Hello` within 10 seconds SHALL be closed.

**Verified by**: `bus::a_socket_instance_connects_once_with_its_token`.

#### Scenario: Token reused
- **WHEN** a socket instance is up and another process connects to its socket with the same token
- **THEN** that connection is closed without `HelloAck` and the instance stays `up`

### Requirement: A crashed or disconnected instance is restarted with backoff
An instance whose process exits on its own, whose connection ends while its process lives, or that has not said `Hello` 30 seconds after it was started SHALL have its process group killed and be started again after a pause of 200 ms doubling up to 5 s, reset after it has run 10 s; `bus.instances` SHALL count the restarts.

**Verified by**: `bus::a_crashed_instance_is_restarted_with_backoff`, `bus::an_instance_that_loses_its_connection_is_restarted`, `bus::an_instance_that_never_says_hello_is_restarted`, `bus::an_instances_own_processes_end_with_it`.

#### Scenario: Crash loop
- **WHEN** the test runtime exits at once three times
- **THEN** the pauses before the restarts are 200, 400 and 800 ms and `restarts` is 3

#### Scenario: Connection closed
- **WHEN** the test runtime closes descriptor 3 but keeps running (`test.close_bus`)
- **THEN** its process is killed and a new process says `Hello`

### Requirement: Stopping, starting and reloading instances
`bus.stop {instance, drain_ms?}` SHALL drain the instance up to `drain_ms`, send SIGTERM to its group and SIGKILL after 3 s, and keep it `stopped` until `bus.start {instance}`, which SHALL answer once it said `Hello`. `bus.reload {instance | module | all}` SHALL restart them one at a time, each drained and waited for until `Hello`, answering old and new pids or `spawn_failed`, `start_failed`, `start_timeout`. A caller acting on itself SHALL get its reply first.

**Verified by**: `bus::an_instance_is_stopped_and_started`, `bus::an_instance_is_stopped_with_sigterm`, `bus::a_module_is_reloaded_one_instance_at_a_time`, `bus::a_reload_answers_a_start_error`, `bus::reloading_a_module_takes_the_right_over_every_instance`, `bus::control_rights_are_checked_for_each_instance`, `bus::a_reload_of_the_server_does_not_wait_for_a_long_drain`.

#### Scenario: No gap for a method
- **WHEN** `t-1` and `t-2` provide `test.echo`, the owner calls it in a loop and `tilr bus reload --module t` runs
- **THEN** no call of the loop fails and both instances have new pids

#### Scenario: A caller reloads itself
- **WHEN** `t-1` calls `bus.reload {"instance": "t-1"}`
- **THEN** it receives the reply before its process is stopped

### Requirement: Reloading bus.toml applies the difference
`bus.reload_config` SHALL read `bus.toml` again, stop removed instances, start added ones, reload instances whose module or entry changed (including `rights`), keep the others running with their pids, keep manually stopped instances stopped, and apply `call_timeout_ms` to new calls.

**Verified by**: `bus::reloading_bus_toml_applies_the_difference`, `bus::call_timeout_ms_of_bus_toml_bounds_every_call`.

#### Scenario: Add, change, remove
- **WHEN** `t-2` is added, `t-1`'s `env` changes and `t-3` is removed, then `tilr bus reload-config` runs
- **THEN** `t-2` is up, `t-1` has a new pid, `t-3` no longer runs, and an unchanged `t-4` keeps its pid

### Requirement: call_timeout_ms bounds every call
The server SHALL hand `call_timeout_ms` of `bus.toml` to the broker at start and on each `bus.reload_config` that applies the file, for the calls made from then on: a call's deadline SHALL be `min(Call.timeout_ms, call_timeout_ms)`, a call that names none SHALL get `call_timeout_ms`, and 0 SHALL mean no default deadline.

**Verified by**: `bus::call_timeout_ms_of_bus_toml_bounds_every_call`.

#### Scenario: Read at start, applied on reload
- **WHEN** `bus.toml` says `call_timeout_ms = 200` and the owner calls `test.sleep 5000`, with no deadline and with 60 s; then the file says 0, `bus.reload_config` runs and the owner calls `test.sleep 400`
- **THEN** both first calls are answered `timeout` and the last `slept`

### Requirement: Methods of configured instances are known
The methods an instance provided since the server started, and those its `rights.provide` names exactly (without `*`), SHALL be known to the broker: a call of one while no live, non-draining provider is connected SHALL be answered `unavailable`, not `not_found`, also before the instance first ran.

**Verified by**: `bus::a_method_of_a_configured_instance_is_unavailable_while_it_is_down`, `bus::an_instance_is_stopped_and_started`.

#### Scenario: A stopped instance's method
- **WHEN** `t-1` provided `test.echo`, it is stopped with `bus.stop`, and the owner calls `test.echo`
- **THEN** the reply is `unavailable`; a call of `nobody.provides` is `not_found`

### Requirement: Listing participants and methods
`bus.instances` SHALL list every configured instance with its name, module, transport, state (`down`, `starting`, `up`, `draining`, `stopped`), pid, restarts and provided methods, and the outside participants connected now with their identity and `client_name`; `bus.methods` SHALL list each method with its kind, lane and the live, non-draining providers.

**Verified by**: `bus::participants_and_methods_are_listed`.

#### Scenario: Draining is not offered
- **WHEN** `t-1` is being drained by `bus.reload` and `bus.methods` is called
- **THEN** `t-1` is not listed as a provider, and `tilrd` is listed for `pane.output` with kind `STREAM` and lane `INTERACTIVE`

### Requirement: An instance's output goes to the server log
Each line an instance writes to stdout or stderr SHALL appear in the server's log as `bus <name>: <line>`.

**Verified by**: `bus::an_instances_output_is_in_the_server_log`.

#### Scenario: A line on stderr
- **WHEN** the test runtime writes `hello log` to stderr
- **THEN** the server log has a line ending with `bus t-1: hello log`
