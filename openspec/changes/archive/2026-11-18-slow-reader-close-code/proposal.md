# Proposal: Which slow reader sees the close code 4001

## Why

The bus closes a participant whose interactive lane is full with code 4001 `too slow`, and
gives the close one deadline (1 s) to go out. The Close frame goes behind what is already
in the socket, so only a reader that takes that in before the deadline sees the code; a
reader that does not read at all sees the connection end without one. The requirement said
every such reader is closed with 4001, and its test read only after the provider had sent
everything, which on a loaded machine was sometimes past the deadline: it failed with no
close code.

## What Changes

- The requirement «Interactive traffic goes ahead of bulk traffic» says that the code 4001
  is seen only by a reader that reads within the close deadline.
- Its scenario «A reader of large interactive chunks stops reading» reads again when the
  server decides to close it; a new scenario «A reader of large interactive chunks never
  reads» sees its connection end without a code.
- The test `bus::a_reader_that_stops_reading_large_interactive_chunks_is_closed_too_slow`
  lets the provider send concurrently and starts the reader reading at the server's event
  `participant too slow, closing`, instead of after the provider is done. The wait for that
  event ends at the latest the harness's timeout after the provider has sent all, so a
  slow machine stretches it and a server that never closes the reader still fails it.
- A new test `bus::a_reader_that_never_reads_sees_its_too_slow_connection_end_without_a_code`
  reads only once the server has let the connection go (its event `bus participant gone`).

## Capabilities

### Modified Capabilities
- `component-bus`: the close code a too-slow reader sees, depending on whether it reads
  within the close deadline.

## Non-goals

- The close deadline and the server's close path are unchanged (product code is not
  touched).

## Impact

`crates/tilrd/tests/bus.rs`, `crates/tilrd/tests/common/bus.rs`, the spec and USE_CASES.md.
Protocol: version 1, no flag. No public API changes.
