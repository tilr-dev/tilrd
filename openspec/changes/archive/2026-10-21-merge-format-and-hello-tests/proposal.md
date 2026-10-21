# Proposal: Check the state formats in a unit test and merge the two Hello timeout tests

## Why

Two slow integration tests repeat checks that a cheaper test covers or can cover:

- `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` waits for the
  server to judge three stub executables in a row (each up to the reload check limit), although
  the reasons a successor is refused for its formats are decided by one pure function that a unit
  test can feed every case at once.
- `auth::a_connection_that_never_says_hello_is_closed_after_10_s` and
  `metrics::a_connection_that_never_says_hello_is_counted_as_no_hello` each wait the real 10 s Hello
  deadline for the same event: a WebSocket that never says `Hello`.

## What Changes

- A table-driven unit test, `reload::tests::check_formats_rejects_every_format_mismatch`, holds every
  way a successor's `state-formats` output is refused: another supervisor or pane state format, no
  `reload-check 1`, no format named, an unparseable format. The integration test keeps the exact
  `state-formats` output of the real executable, one stub (another pane state format) and the
  reload into a suitable executable afterwards.
- One test, `auth::a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello`,
  holds every assertion of both Hello tests: close code 4002 `timeout` after 9 to 13 s, the log line,
  a client that says `Hello` after a second stays, and only `no_hello` is counted. The metrics test
  binary no longer waits for the Hello deadline; the `/metrics` scrape helpers move to the shared test
  harness so both binaries use them.
- Only the `**Verified by**` lines of three requirements change, to name these tests:
  reload "The successor must run and read the state formats", access-control "A handshake has 10
  seconds" and metrics "Clients let go, by reason". No requirement text or scenario changes.

## Non-goals

- No change to the server's behaviour, its log lines, close codes or metrics.
- No shortcut for the 10 s Hello deadline: the merged test still waits for it.
- `reload::a_broken_executable_is_not_reloaded_into` and the itests cases are unchanged.

## Impact

Tests (`crates/tilrd/src/reload.rs`, `crates/tilrd/tests/reload.rs`, `tests/metrics.rs`,
`tests/auth.rs`, `tests/common/`), `USE_CASES.md` and the `**Verified by**` lines of the specs above. Not breaking;
nothing is handed over on a reload or an update. The itests `@case(spec=[...])` tags name
requirements, whose names do not change, so no tag changes.
