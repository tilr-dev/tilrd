## ADDED Requirements

### Requirement: The libraries connect on every transport
`Bus.connect()` in the Deno library (`sdk/deno`, also used in browser pages) and the Python library (`sdk/python`) SHALL use `TILR_BUS_FD` when set, else `TILR_BUS_SOCK` with `TILR_BUS_TOKEN`, else a WebSocket URL with a credential (arguments, or `TILR_BUS_URL` and `TILR_BUS_CREDENTIAL`), send `Hello` with every method provided before it, and expose `HelloAck.identity` as `identity`; with none of them it SHALL fail with `unavailable` naming the variables.

**Verified by**: `sdk/deno/test/bus_test.ts::connects_over_fd_socket_and_websocket`, `sdk/python/tests/test_bus.py::test_connects_over_fd_socket_and_websocket`.

#### Scenario: Three transports
- **WHEN** `bus.toml` runs the library's test runtime as `d-fd` (`connect = "fd"`) and `d-sock` (`connect = "socket"`), and a third copy connects over WebSocket with a minted token named `d-ws`
- **THEN** `tilr bus call test.whoami --target instance:d-fd` prints `instance:d-fd`, and the same for `instance:d-sock` and `token:d-ws`

### Requirement: Calls and replies carry bytes unchanged
`call` SHALL send its payload and `content_type` as given and return the reply's bytes unchanged; a handler's returned bytes (or `{payload, contentType}`) SHALL be the reply.

**Verified by**: `sdk/deno/test/bus_test.ts::bytes_pass_unchanged`, `sdk/python/tests/test_bus.py::test_bytes_pass_unchanged`.

#### Scenario: Non-UTF-8 payload
- **WHEN** a test calls the runtime's `test.echo` with the bytes 0x00 0xff 0xfe and `content_type` `application/octet-stream`
- **THEN** the reply has the same three bytes and the same `content_type`

### Requirement: Errors keep their codes
A handler that throws `BusError(code, message)` SHALL make the reply that error; any other exception SHALL reply `handler_error` with the exception's type and message and no traceback; a broker error SHALL be raised to the caller as `BusError` with the broker's code and message; an incoming call for a method not provided SHALL be answered `not_supported`.

**Verified by**: `sdk/deno/test/bus_test.ts::errors_keep_their_codes`, `sdk/python/tests/test_bus.py::test_errors_keep_their_codes`.

#### Scenario: Thrown and broker errors
- **WHEN** the runtime's `test.fail` throws `BusError("bad_request", "x")`, `test.crash` throws a plain error, and a caller calls `nobody.here`
- **THEN** the callers get `bad_request: x`, `handler_error: <type>: <message>`, and `BusError` with `not_found`

### Requirement: A waiting handler does not stop the others
Each incoming call SHALL run as its own task, so a handler awaiting a nested call SHALL NOT delay other calls, and a chain back into the same participant SHALL complete.

**Verified by**: `sdk/deno/test/bus_test.ts::nested_and_crossing_calls_complete`, `sdk/python/tests/test_bus.py::test_nested_and_crossing_calls_complete`, `sdk/python/tests/test_bus.py::test_python_calls_deno_and_back`.

#### Scenario: Crossing chain
- **WHEN** `py-1` handles a call by calling `d-1`, which calls `py-1` again
- **THEN** the outer call is answered and `py-1` served another call meanwhile

### Requirement: Events can be emitted and received
`emit(subject, payload)` SHALL send an `Event`; `on(pattern, handler)` SHALL subscribe, call the handler for each event with its subject, payload and `from`, and return a way to unsubscribe; a denied subscription SHALL fail with `BusError` `denied`.

**Verified by**: `sdk/deno/test/bus_test.ts::events_are_emitted_and_received`, `sdk/python/tests/test_bus.py::test_events_are_emitted_and_received`.

#### Scenario: Python hears Deno
- **WHEN** `py-1` subscribes to `test.*` and `d-1` emits `test.ping` with `1`
- **THEN** `py-1`'s handler receives subject `test.ping`, payload `1` and `from` `instance:d-1`

### Requirement: Streams can be read and served with credit
`stream(method, payload, {window})` SHALL return an async iterator of chunk payloads that returns credit as each chunk is consumed and cancels the stream when the iteration is left early; a provided stream handler SHALL get a writer whose `write` waits until credit allows the bytes, and whose return ends the stream with `End`.

**Verified by**: `sdk/deno/test/bus_test.ts::streams_are_read_and_served_with_credit`, `sdk/python/tests/test_bus.py::test_streams_are_read_and_served_with_credit`.

#### Scenario: Slow consumer
- **WHEN** `d-1` serves `test.flood` writing 64 MiB and `py-1` reads it with a 64 KiB window, sleeping 10 ms per chunk
- **THEN** `py-1` receives all 64 MiB in order and `d-1`'s process memory stays under 64 MiB

#### Scenario: Early break
- **WHEN** a reader leaves the loop after one chunk
- **THEN** the provider's handler sees the cancellation

### Requirement: The core's methods have typed helpers
The libraries SHALL include the generated types of `crates/tilr-bus/proto/tilr/` and helpers for the core's methods and events (`pane.create`, `pane.input`, `pane.read`, `pane.output`, `pane.*` events), so a script can create a pane, type into it and read its screen without encoding protobuf by hand.

**Verified by**: `sdk/deno/test/bus_test.ts::a_script_drives_a_terminal`, `sdk/python/tests/test_bus.py::test_a_script_drives_a_terminal`.

#### Scenario: Python drives a terminal
- **WHEN** a Python instance with `pane = ["profile:demo", "access:own"]` creates a pane with profile `demo`, inputs `echo sum $((6*7))` and Enter, and reads with `pane.read` until a line is `sum 42`
- **THEN** it finds that line and `pane.exited` arrives after it closes the pane

### Requirement: Deadlines and cancellation reach the handler
A handler SHALL see the time its caller has left (`ctx.deadline`), `ctx.call` SHALL pass the trace id and the remaining time on, and a `Cancel` for its call SHALL abort the handler (Deno `ctx.signal`, Python task cancellation) without a reply.

**Verified by**: `sdk/deno/test/bus_test.ts::a_cancel_aborts_the_handler`, `sdk/python/tests/test_bus.py::test_a_cancel_aborts_the_handler`.

#### Scenario: Caller times out
- **WHEN** a caller calls `test.sleep` (10 s, records whether it was aborted) with `timeout_ms` 300
- **THEN** the caller gets `timeout`, and the runtime records the abort within a second

### Requirement: The libraries respect the broker's limits
A library SHALL wait for the socket to drain before writing more, keep at most `HelloAck.max_in_flight` own calls pending (later calls wait), and refuse a call larger than `HelloAck.max_frame` with `too_large` without sending it.

**Verified by**: `sdk/deno/test/bus_test.ts::limits_are_respected`, `sdk/python/tests/test_bus.py::test_limits_are_respected`.

#### Scenario: Many calls at once
- **WHEN** a runtime starts 1000 calls to a slow method at once
- **THEN** none is answered `busy` for the caller's in-flight limit and all complete

#### Scenario: Too large
- **WHEN** a runtime calls with a 17 MiB payload
- **THEN** the call fails with `too_large` and the connection stays up

### Requirement: A closed connection ends every pending call
When the connection to the broker closes, every pending call and stream SHALL fail with `unavailable` and `closed` (Deno) / `wait_closed()` (Python) SHALL complete; a library SHALL keep the latest `CredentialUpdate` as its credential.

**Verified by**: `sdk/deno/test/bus_test.ts::a_closed_connection_ends_pending_calls`, `sdk/python/tests/test_bus.py::test_a_closed_connection_ends_pending_calls`.

#### Scenario: Broker gone
- **WHEN** a runtime has a call pending and tilrd is killed with SIGKILL
- **THEN** the call fails with `unavailable` and the runtime's closed signal completes

### Requirement: The generated schema matches the protocol
`scripts/gen-bus-sdk.sh --check` SHALL exit 0 only when the code checked in under `sdk/` is what `crates/tilr-bus/proto/` generates, and CI's `sdk` job SHALL run it.

**Verified by**: CI job `sdk` (`scripts/gen-bus-sdk.sh --check`).

#### Scenario: Schema edited without regenerating
- **WHEN** a field is added to `bus.proto` and the generated files are not regenerated
- **THEN** `scripts/gen-bus-sdk.sh --check` prints the differing file and exits 1
