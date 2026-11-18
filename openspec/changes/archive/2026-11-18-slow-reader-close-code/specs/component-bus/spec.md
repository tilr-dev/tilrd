## MODIFIED Requirements

### Requirement: Interactive traffic goes ahead of bulk traffic
Each connection's outbound queue SHALL have an interactive lane (replies and chunks of `INTERACTIVE` methods, events, credits, cancels) of at most 1024 envelopes or 32 MiB unwritten, and a bulk lane (chunks and replies of `BULK` methods) of at most 1024 envelopes or 4 MiB; the bulk lane MUST be written only when the interactive lane is empty. A connection whose interactive lane is at either bound when an envelope must be queued SHALL be closed 4001 `too slow`, seen only if it reads within 1 s.

**Verified by**: `bus::interactive_traffic_goes_ahead_of_bulk_traffic`, `bus::a_reader_that_stops_reading_large_interactive_chunks_is_closed_too_slow`, `bus::a_reader_that_never_reads_sees_its_too_slow_connection_end_without_a_code`, unit test `interactive_packets_go_ahead_of_bulk_ones` (crates/tilrd/src/outbox.rs).

#### Scenario: Echo during a bulk stream
- **WHEN** a WebSocket participant reads a 64 MiB `test.bulk` stream and calls `test.echo` meanwhile
- **THEN** each echo is answered before the next bulk megabyte reaches it

#### Scenario: A reader of large interactive chunks stops reading
- **WHEN** a reader opens three interactive streams with 16 MiB windows and reads nothing while their providers send 48 MiB in chunks of 1 MiB, and reads again when the server decides to close it
- **THEN** it receives the Close frame with code 4001 before all of it reached it

#### Scenario: A reader of large interactive chunks never reads
- **WHEN** the same reader reads nothing until the server, past the close deadline of 1 s, has let its connection go
- **THEN** its connection ends without a close code, before all of it reached it
