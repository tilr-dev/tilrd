## ADDED Requirements

### Requirement: The libraries connect as the instance they were started as
`connect()` in the Deno and the Python library SHALL use `TILR_BUS_FD` when set, else `TILR_BUS_SOCK` with `TILR_BUS_TOKEN`, send `Hello` with every method registered before it, and expose the name from `HelloAck` as `instance`; with neither variable set it SHALL fail with `unavailable` naming the variables.

**Verified by**: `sdk/deno/test/bus_test.ts::connects_over_fd_and_socket`, `sdk/python/tests/test_bus.py::test_connects_over_fd_and_socket`.

#### Scenario: Both transports
- **WHEN** `bus.toml` runs the library's test runtime as `d-fd` (`connect = "fd"`) and `d-sock` (`connect = "socket"`)
- **THEN** `tilr bus call test.whoami --target d-fd` prints `d-fd`, and the same for `d-sock`

### Requirement: Calls and replies carry bytes unchanged
`call` SHALL send its payload and `content_type` as given and return the reply's bytes unchanged; a handler's returned bytes (or `{payload, contentType}`) SHALL be the reply.

**Verified by**: `sdk/deno/test/bus_test.ts::bytes_pass_unchanged`, `sdk/python/tests/test_bus.py::test_bytes_pass_unchanged`.

#### Scenario: Non-UTF-8 payload
- **WHEN** a test calls the runtime's `test.echo` with the bytes 0x00 0xff 0xfe and `content_type` `application/octet-stream`
- **THEN** the reply has the same three bytes and the same `content_type`

### Requirement: Errors keep their codes
A handler that throws `BusError(code, message)` SHALL make the reply that error; any other exception SHALL reply `handler_error` with the exception's type and message and no traceback; a broker error SHALL be raised to the caller as `BusError` with the broker's code and message.

**Verified by**: `sdk/deno/test/bus_test.ts::errors_keep_their_codes`, `sdk/python/tests/test_bus.py::test_errors_keep_their_codes`.

#### Scenario: Thrown and broker errors
- **WHEN** the runtime's `test.fail` throws `BusError("bad_request", "x")`, `test.crash` throws a plain error, and a caller calls `nobody.here`
- **THEN** the callers get `bad_request: x`, `handler_error: <type>: <message>`, and `BusError` with `not_found`

### Requirement: A waiting handler does not stop the others
Each incoming call SHALL run as its own task, so a handler awaiting a nested call SHALL NOT delay other calls, and a chain back into the same instance SHALL complete.

**Verified by**: `sdk/deno/test/bus_test.ts::nested_and_crossing_calls_complete`, `sdk/python/tests/test_bus.py::test_nested_and_crossing_calls_complete`, `sdk/python/tests/test_bus.py::test_python_calls_deno_and_back`.

#### Scenario: Crossing chain
- **WHEN** `py-1` handles a call by calling `d-1`, which calls `py-1` again
- **THEN** the outer call is answered and `py-1` served another call meanwhile

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
- **THEN** none is answered `busy` by the broker for the caller's in-flight limit and all complete

#### Scenario: Too large
- **WHEN** a runtime calls with a 17 MiB payload
- **THEN** the call fails with `too_large` and the connection stays up

### Requirement: A closed connection ends every pending call
When the connection to the broker closes, every pending call SHALL fail with `unavailable` and `closed` (Deno) / `wait_closed()` (Python) SHALL complete.

**Verified by**: `sdk/deno/test/bus_test.ts::a_closed_connection_ends_pending_calls`, `sdk/python/tests/test_bus.py::test_a_closed_connection_ends_pending_calls`.

#### Scenario: Broker gone
- **WHEN** a runtime has a call pending and tilrd is killed with SIGKILL
- **THEN** the call fails with `unavailable` and the runtime's closed signal completes

### Requirement: The generated schema matches bus.proto
`scripts/gen-bus-sdk.sh --check` SHALL exit 0 only when the code checked in under `sdk/` is what `crates/tilr-bus/proto/bus.proto` generates, and CI's `sdk` job SHALL run it.

**Verified by**: CI job `sdk` (`scripts/gen-bus-sdk.sh --check`).

#### Scenario: Schema edited without regenerating
- **WHEN** a field is added to `bus.proto` and the generated files are not regenerated
- **THEN** `scripts/gen-bus-sdk.sh --check` prints the differing file and exits 1
