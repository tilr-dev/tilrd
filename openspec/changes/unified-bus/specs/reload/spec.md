## MODIFIED Requirements

### Requirement: state-formats names what this executable reads
`tilrd state-formats` SHALL print one `name version` line per format it reads: `supervisor-state 3`, `pane-state 3`, `bus-state 1` and `unix-socket 1` at HEAD, followed by `reload-check 1`: it answers the check of a successor.

**Verified by**: `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`; itests `UC-067` (`uc_067.py::a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on`).

#### Scenario: Formats of this build
- **WHEN** `tilrd state-formats` runs
- **THEN** it prints `supervisor-state 3\npane-state 3\nbus-state 1\nunix-socket 1\nreload-check 1\n`

#### Scenario: From the build before the bus
- **WHEN** a server of the build before this change (writing `supervisor-state 2`) is asked to reload into this build
- **THEN** it does not reload, logs why, and keeps its panes and clients

### Requirement: Client connections end and clients reconnect
A reload SHALL end every WebSocket connection (owner, `tilr`, Tilr Web, outside participants; their sockets are not carried over) while instance connections carry over; a client that reconnects SHALL find its panes as they were. Input admitted to the client's delivery queue SHALL reach the pane after reconnection, including input the supervisor had not read before exec. Admitted input SHALL retain its order without loss; replay MAY duplicate input applied but not acknowledged before reload.

**Verified by**: `input_delivery::unread_input_is_replayed_after_reload_in_the_middle_of_a_batch`, `reload::bus_instances_survive_a_reload`; itests `UC-063` (`uc_063.py::a_client_typing_through_an_update_reconnects_and_nothing_typed_is_lost_or_doubled`).

#### Scenario: Typing through an update
- **WHEN** a client types numbered lines into `cat >>typed` through an update
- **THEN** after reconnection every admitted line is in the file in order, allowing replay of input not acknowledged before reload

#### Scenario: Streams end and are reopened
- **WHEN** a client reads `pane.output` and the server reloads
- **THEN** its connection ends, and after reconnecting a new `pane.output` starts with a full frame of the same screen
