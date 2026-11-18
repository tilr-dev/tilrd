# bus-sdks Specification

## Purpose
Client libraries of the bus `tilr.bus.v1` for the participants that are not written in Rust:
`sdk/deno` for Deno and the browser, `sdk/python` for Python. Each connects on every transport
the server offers, calls, provides methods and streams with credit, emits and subscribes to
events, drives panes, and carries the bus schemas as code generated from
`crates/tilr-bus/proto/`, which CI checks against the schemas.

## Requirements

### Requirement: The libraries connect on every transport
`Bus.connect()` in the Deno library (`sdk/deno`, also used in browser pages) and the Python library (`sdk/python`) SHALL connect over an inherited descriptor, an instance socket with its token, or a WebSocket URL with a credential or the access token, as the next requirement picks one. It SHALL send `Hello` with every method provided before it and expose `HelloAck.identity` as `identity`; with no transport named it SHALL fail with `unavailable` naming the variables.

**Verified by**: `sdk/deno/test/bus_test.ts::connects_over_fd_socket_and_websocket`, `sdk/python/tests/test_bus.py::test_connects_over_fd_socket_and_websocket` (scenario "Each transport"); `sdk/deno/test/bus_test.ts::instances_and_a_token_participant_connect_over_fd_socket_and_websocket`, `sdk/python/tests/test_bus.py::test_instances_and_a_token_participant_connect_over_fd_socket_and_websocket` (scenario "Three transports").

#### Scenario: Each transport
- **WHEN** the library connects over a descriptor, an instance socket and a WebSocket with a credential, each to a fake broker, and as the owner to a real tilrd over its Unix socket and over TCP with the access token
- **THEN** each broker gets `Hello` (with the token or credential it was given) and the library's `identity` is the one `HelloAck` named

#### Scenario: Three transports
- **WHEN** `bus.toml` runs the library's test runtime as `d-fd` (`connect = "fd"`) and `d-sock` (`connect = "socket"`), and a third copy connects over WebSocket with a minted token named `d-ws`
- **THEN** `tilr bus call test.whoami --target instance:d-fd` prints `instance:d-fd`, and the same for `instance:d-sock` and `token:d-ws`

### Requirement: The arguments pick the transport before the environment
The transport the arguments of `Bus` name SHALL beat the environment; without one, the libraries SHALL take the environment's in this order: `TILR_BUS_FD` when set, else `TILR_BUS_SOCK` with `TILR_BUS_TOKEN`, else `TILR_BUS_URL` with `TILR_BUS_CREDENTIAL`, an empty variable counting as unset. A connect that failed SHALL leave the library ready to connect again.

**Verified by**: `sdk/deno/test/bus_test.ts::connects_over_fd_socket_and_websocket`, `sdk/python/tests/test_bus.py::test_connects_over_fd_socket_and_websocket`.

#### Scenario: The arguments beat the environment
- **WHEN** the environment sets `TILR_BUS_FD`, `TILR_BUS_SOCK` with `TILR_BUS_TOKEN` and `TILR_BUS_URL`, and `connect` is called with a WebSocket URL
- **THEN** the library connects to that URL; without it, over the descriptor; with `TILR_BUS_FD` empty, over the instance socket

#### Scenario: Connect again after a failure
- **WHEN** `connect` fails because nothing names a transport, and is called again on the same object with an instance socket
- **THEN** the second connect succeeds

### Requirement: Calls and replies carry bytes unchanged
`call` SHALL send its payload and `content_type` as given and return the reply's bytes unchanged; a handler's returned bytes (or `{payload, contentType}`) SHALL be the reply. A 64-bit field of a generated message SHALL keep every bit (Deno: `bigint`; Python: `int`).

**Verified by**: `sdk/deno/test/bus_test.ts::bytes_pass_unchanged`, `sdk/python/tests/test_bus.py::test_bytes_pass_unchanged`.

#### Scenario: Non-UTF-8 payload
- **WHEN** a test calls the runtime's `test.echo` with the bytes 0x00 0xff 0xfe and `content_type` `application/octet-stream`
- **THEN** the reply has the same three bytes and the same `content_type`

#### Scenario: A nonce past 2^53
- **WHEN** a test calls `bus.ping` with the nonce 2^64 - 3
- **THEN** the reply's nonce is 2^64 - 3

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
`emit(subject, payload)` SHALL send an `Event`; `on(pattern, handler)` SHALL subscribe, call the handler for each event with its subject, payload and `from`, one event at a time and in order, and return a way to unsubscribe; a denied subscription SHALL fail with `BusError` `denied`.

**Verified by**: `sdk/deno/test/bus_test.ts::events_are_emitted_and_received`, `sdk/python/tests/test_bus.py::test_events_are_emitted_and_received`.

#### Scenario: Python hears Deno
- **WHEN** `py-1` subscribes to `test.*` and `d-1` emits `test.ping` with `1`
- **THEN** `py-1`'s handler receives subject `test.ping`, payload `1` and `from` `instance:d-1`

### Requirement: A subscriber that is behind drops events
While a subscription's handler is busy, the library SHALL hold at most 1024 events for it (`SUBSCRIPTION_QUEUE`); an event that arrives when 1024 wait SHALL be dropped and counted in the subscription's `dropped`, and the connection and the subscription SHALL stay, the events held handed to the handler in order, as the broker drops events for a subscriber that does not read.

**Verified by**: `sdk/deno/test/bus_test.ts::a_subscriber_that_is_behind_drops_events`, `sdk/python/tests/test_bus.py::test_a_subscriber_that_is_behind_drops_events`.

#### Scenario: A handler that waits
- **WHEN** a handler waits on its first event while 1029 more arrive, and is then released
- **THEN** `dropped` is 5, the handler gets the first event and the next 1024 in order, and an event sent after it is delivered too

### Requirement: Streams can be read and served with credit
`stream(method, payload, {window})` SHALL return an async iterator of chunk payloads whose `Call` is sent when the iteration begins (in both libraries), that returns credit as each chunk is consumed, and that cancels the stream when it is left early (next requirement); a `Cancel` of a stream SHALL never be sent before its `Call`. A provided stream handler SHALL get a writer whose `write` waits until credit allows the bytes, and whose return ends the stream with `End`.

**Verified by**: `sdk/deno/test/bus_test.ts::streams_are_read_and_served_with_credit`, `sdk/python/tests/test_bus.py::test_streams_are_read_and_served_with_credit`.

#### Scenario: A reader that stops
- **WHEN** `d-1` serves `test.flood` writing 64 MiB in chunks of 64 KiB and a reader with a 64 KiB window takes one chunk and stops reading
- **THEN** `test.flood_state` answers once `d-1`'s `write` waits for credit, having sent no more than the window beyond the credit it got back; the reader then receives all 64 MiB in order and `d-1` was never more than the window ahead of its credit

#### Scenario: Early break
- **WHEN** a reader leaves the loop (Deno) or the `async with` block (Python) after one chunk
- **THEN** the provider's handler sees the cancellation

### Requirement: Leaving a stream early cancels it
In Deno, a `for await` loop over a stream left by `break`, `return` or an exception SHALL cancel the stream. In Python, leaving an `async with` block around the iteration (`async with bus.stream(...) as chunks:`) SHALL cancel it at once; a plain `async for` left early cancels only when the stream object is collected, as best effort.

**Verified by**: `sdk/deno/test/bus_test.ts::streams_are_read_and_served_with_credit`, `sdk/python/tests/test_bus.py::test_streams_are_read_and_served_with_credit`, `sdk/python/tests/test_bus.py::test_leaving_async_with_cancels_the_stream`.

#### Scenario: Leaving the block frees the slot at once
- **WHEN** a Python reader with `max_in_flight` 1 leaves `async with bus.stream(...)` after one chunk, still holding the stream, and makes a call
- **THEN** the broker gets the stream's `Cancel` and then the call, within 2 s

### Requirement: The core's methods have typed helpers
The libraries SHALL include the generated types of `crates/tilr-bus/proto/tilr/` and helpers for the core's methods and events (`pane.create`, `pane.input`, `pane.read`, `pane.output`, `pane.*` events), so a script can create a pane, type into it and read its screen without encoding protobuf by hand.

**Verified by**: `sdk/deno/test/bus_test.ts::pane_helpers_speak_the_pane_schema`, `sdk/python/tests/test_bus.py::test_pane_helpers_speak_the_pane_schema` (the helpers' requests and decoded replies and events against a fake broker); `sdk/deno/test/bus_test.ts::a_script_drives_a_terminal`, `sdk/python/tests/test_bus.py::test_a_script_drives_a_terminal` (scenario "Python drives a terminal").

#### Scenario: The helpers speak the pane schema
- **WHEN** a script calls `pane.create` with profile `demo`, types a line and reads the screen through a fake broker
- **THEN** the broker gets a `CreateRequest` with profile `demo`, the line as `RawInput` and then the Enter key, and the script gets the `PaneInfo`, the `ReadReply` lines and the decoded `pane.exited` event

#### Scenario: Python drives a terminal
- **WHEN** a Python script whose token holds `call = ["pane.*"]`, `subscribe = ["pane.exited"]` and `pane = ["profile:demo", "access:own"]` connects over WebSocket, creates a pane with profile `demo`, inputs `echo sum $((6*7))` and Enter, and reads with `pane.read` until a line is `sum 42`
- **THEN** it finds that line and `pane.exited` arrives after it closes the pane

### Requirement: Deadlines and cancellation reach the handler
A handler SHALL see the time its caller has left (`ctx.deadline`), `ctx.call` SHALL pass the trace id and the remaining time on, and a `Cancel` for its call SHALL abort the handler (Deno `ctx.signal`, Python task cancellation) without a reply.

**Verified by**: `sdk/deno/test/bus_test.ts::a_cancel_aborts_the_handler`, `sdk/python/tests/test_bus.py::test_a_cancel_aborts_the_handler`, `sdk/deno/test/bus_test.ts::a_cancelled_call_gets_no_reply`, `sdk/python/tests/test_bus.py::test_a_cancelled_call_gets_no_reply`.

#### Scenario: Caller times out
- **WHEN** a caller calls `test.sleep` (10 s, records whether it was aborted) with `timeout_ms` 300
- **THEN** the caller gets `timeout`, and the runtime records the abort within a second

#### Scenario: No reply after a Cancel
- **WHEN** a fake broker sends two calls, a `Cancel` for each (one handler stops when cancelled, the other answers anyway), and then a third call
- **THEN** the next envelope the library sends is the third call's reply

### Requirement: The libraries respect the broker's limits
A library SHALL wait for the socket to drain before writing more (in Deno with one wait for all writes that wait, adding no listener per write), keep at most `HelloAck.max_in_flight` own calls pending (later calls wait), and refuse a call larger than `HelloAck.max_frame` with `too_large` without sending it.

**Verified by**: `sdk/deno/test/bus_test.ts::limits_are_respected`, `sdk/python/tests/test_bus.py::test_limits_are_respected` (against a real tilrd, which answers `busy` past 256 pending calls of a connection, and a fake broker with `max_in_flight` 3), `sdk/deno/test/bus_test.ts::backpressured_writes_add_no_listeners`.

#### Scenario: Many calls at once
- **WHEN** a runtime starts 1000 calls to a slow method of a real tilrd at once
- **THEN** none is answered `busy` for the caller's in-flight limit and all complete

#### Scenario: Writes that wait for the socket
- **WHEN** the Deno library writes 66 envelopes of 1 MiB to a stream socket, each over the socket's buffer, two of them at once
- **THEN** the socket has as many `close` and `drain` listeners as before

#### Scenario: Too large
- **WHEN** a runtime calls with a 17 MiB payload
- **THEN** the call fails with `too_large` and the connection stays up

### Requirement: A closed connection ends every pending call
When the connection to the broker closes, every pending call and stream SHALL fail with `unavailable` and `closed` (Deno) / `wait_closed()` (Python) SHALL complete; a `ConnectionError` followed by the close SHALL end the connection with both, the error's code for the calls and the close's code in the close information. A library SHALL keep the latest `CredentialUpdate` as its credential.

**Verified by**: `sdk/deno/test/bus_test.ts::a_closed_connection_ends_pending_calls`, `sdk/python/tests/test_bus.py::test_a_closed_connection_ends_pending_calls`.

#### Scenario: Broker gone
- **WHEN** a runtime has a call pending and tilrd is killed with SIGKILL
- **THEN** the call fails with `unavailable` and the runtime's closed signal completes

#### Scenario: An error, then the close
- **WHEN** a fake broker sends `ConnectionError{too_slow}` and closes the WebSocket with 4001
- **THEN** the closed signal has code 4001 and the error `too_slow`, and a later call fails `too_slow`

### Requirement: The Deno library builds no code at run time
The Deno library, its generated schema code and every npm package its module graph reaches SHALL contain no `eval(` and no `Function(`, so a page with a Content-Security-Policy without `unsafe-eval` can run it; the generated code SHALL be protobuf-es's static code (`@bufbuild/protobuf`, pinned in `sdk/deno/deno.json` and `deno.lock`).

**Verified by**: CI job `sdk` (`sdk/deno/tools/check_no_eval.ts`, run by `scripts/sdk-check.sh`).

#### Scenario: A dependency that evaluates code
- **WHEN** a module of the library's graph contains `new Function(`
- **THEN** `scripts/sdk-check.sh` prints its file and line and exits 1

### Requirement: The generated schema matches the protocol
`scripts/gen-bus-sdk.sh --check` SHALL exit 0 only when the code checked in under `sdk/` is what `crates/tilr-bus/proto/` generates, and CI's `sdk` job SHALL run it.

**Verified by**: CI job `sdk` (`scripts/gen-bus-sdk.sh --check`).

#### Scenario: Schema edited without regenerating
- **WHEN** a field is added to `bus.proto` and the generated files are not regenerated
- **THEN** `scripts/gen-bus-sdk.sh --check` prints the differing file and exits 1
