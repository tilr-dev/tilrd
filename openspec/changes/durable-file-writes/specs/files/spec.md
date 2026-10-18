## MODIFIED Requirements

### Requirement: Writes are whole or not at all
`FsWrite` SHALL take chunks of one `transfer_id` (path, `total_size`, `create`, `truncate` in the first) and answer one `FsAck` after the last chunk or as soon as it is refused (the rest then ignored). The server MUST write a temporary file next to the target and rename it over once `last` came with exactly `total_size` bytes: no reader sees half a file. Before an `FsAck` with an empty error the server SHALL have synced the temporary file's data to disk before the rename and the target's directory after it.

**Verified by**: `files::a_file_is_written_whole_or_not_at_all`, UC-070 (`uc_070.py::files_of_zero_one_and_exactly_the_limit_go_both_ways_intact`), UC-046 (`uc_046.py::a_finished_write_is_synced_before_it_is_acknowledged`).

#### Scenario: A new file in chunks
- **WHEN** a client writes 2 x 256 KiB + 777 bytes to the missing `~/notes.md` with `create`
- **THEN** one `FsAck` with an empty error comes and the file holds those bytes

#### Scenario: Wrong amount of data
- **WHEN** a write announces `total_size` 100 and sends 50 bytes, or 150, with `last`
- **THEN** the ack's error contains `the write ended after 50 of 100 bytes`, respectively `carries more data than it announced`, and the file is unchanged

#### Scenario: Synced before the answer
- **WHEN** a client writes `~/notes.md` and the server's system calls are traced
- **THEN** an `fsync` of the temporary file comes before the `rename` onto `~/notes.md`, an `fsync` of the home directory after it, and both before the `FsAck`
