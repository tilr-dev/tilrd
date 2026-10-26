## ADDED Requirements

### Requirement: A superseded attach ends
When the server closes an attached `tilr`'s connection with close code 4000 (`superseded`), `tilr` SHALL NOT reconnect: it SHALL restore the terminal, print `[superseded by another connection of this client]` on stdout and exit with status 0.

**Verified by**: `client::an_attach_superseded_by_its_own_client_id_ends`.

#### Scenario: Another connection with the same client id
- **WHEN** `tilr` is attached and a second connection says `Hello` with `tilr`'s `client_id` and the same token on the same transport
- **THEN** `tilr` prints `[superseded by another connection of this client]` and exits 0 without connecting again
- **AND** the pane goes on

## MODIFIED Requirements

### Requirement: How an attach ends
An attach SHALL end with one line on stdout and exit status 0: `[detached from pane XXXXXXXX]` (the first 8 characters of the id) on Ctrl+] d or end of input, `[pane XXXXXXXX exited with code N]` on `PaneExited`, `[pane XXXXXXXX is gone]` when a `PaneList` lacks the pane or it is gone after a reconnect, `[tilrd stopped]` when the local server stopped, and `[superseded by another connection of this client]` when the server closed the connection as superseded (4000).

**Verified by**: `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches`, itests `UC-016` (`uc_016.py::tilr_attaches_to_a_tilrd_running_in_the_foreground`), `UC-017` (`uc_017.py::kill_server_leaves_no_process_and_no_socket`), `client::an_attach_superseded_by_its_own_client_id_ends`.

#### Scenario: Detach
- **WHEN** the user presses Ctrl+] then d
- **THEN** `tilr` prints `[detached from pane …]`, exits 0 and the pane goes on

#### Scenario: Server stopped while attached
- **WHEN** the tilrd an attached `tilr` uses stops on SIGTERM
- **THEN** `tilr` prints `[tilrd stopped]` and exits 0

### Requirement: Reconnect while attached
An attached client SHALL ping every 5 s and SHALL take 15 s without hearing anything (pongs included), a failed send or a read error as a lost connection (a 4000 close ends the attach instead). It SHALL then show `tilr: disconnected (<why>), reconnecting… (Ctrl+] d detaches)` in reverse video on the bottom row, drop keys typed while disconnected (except the prefix and its `d`), and connect again with the same `client_id`, 0.1 s after the loss and then with waits doubling up to 2 s.

**Verified by**: `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches`, itests `UC-014` (`uc_014.py::after_a_sigkill_the_next_tilr_replaces_the_socket_and_starts_a_new_daemon`).

#### Scenario: Reload of the server
- **WHEN** the server reloads while `tilr` is attached
- **THEN** `tilr` reconnects, checks `ListPanes` for its pane, subscribes again, and input typed afterwards reaches the pane

#### Scenario: SIGKILLed server
- **WHEN** the local tilrd is killed with SIGKILL, so its socket refuses and its lock is free
- **THEN** the attached `tilr` prints `[tilrd stopped]` and exits 0

#### Scenario: Failed attempt
- **WHEN** a reconnect attempt fails for another reason
- **THEN** the status line shows the error and the attempt number, and the next attempt waits twice as long, at most 2 s
