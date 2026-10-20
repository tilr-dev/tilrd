## MODIFIED Requirements

### Requirement: Protocol saves are one change, and watches can stop
A save through `FsWrite` in a watched directory SHALL be reported as one MODIFY of the file, and the temporary file it went through MUST NOT be reported. A second `FsWatch` of the same path replaces the first. `FsUnwatch`, naming the path as in its watch, SHALL stop it and answer `FsAck`, or `<path>: not watched`. Watches end with the connection.

**Verified by**: `files::a_watched_directory_reports_what_changes_in_it` and `files::a_watched_save_can_be_reported_before_its_ack`.

#### Scenario: Save through the protocol
- **WHEN** a client watching `~/watched` writes `~/watched/old.txt` with `FsWrite`
- **THEN** it gets one `FsChanged` MODIFY for `old.txt` and none for a `.tilr-write-` file

#### Scenario: Change arrives before the save acknowledgement
- **WHEN** a successful watched save is held after replacing the file and the watcher queues its notification before the save acknowledgement
- **THEN** the client receives both the unsolicited `FsChanged` MODIFY and the `FsAck` carrying the write's request id; the test retains the notification while awaiting the acknowledgement

#### Scenario: After unwatching
- **WHEN** the client sends `FsUnwatch ~/watched` and a file is then created there
- **THEN** no `FsChanged` comes for it, and a second `FsUnwatch` answers `~/watched: not watched`
