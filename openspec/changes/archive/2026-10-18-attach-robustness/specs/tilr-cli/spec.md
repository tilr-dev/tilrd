## MODIFIED Requirements

### Requirement: Attached terminal
Attaching SHALL need stdin to be a terminal, else fail with ``attaching needs a terminal (`tilr ls`, `tilr new -d` do not)``. Attached, the terminal SHALL be in raw mode on the alternate screen with line wrap off and bracketed paste on, restored on every way out. On each (re)connection the client SHALL send `Resize` with the terminal's size and then `Subscribe` for the pane, draw every `Frame` of that pane, and send `Resize` when the terminal is resized.

**Verified by**: `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches`, itests `UC-010` (`uc_010.py::tilr_without_a_running_daemon_starts_tilrd_detached_and_connects`).

#### Scenario: Shell in a pane
- **WHEN** no tilrd runs and the user starts `tilr` in a terminal
- **THEN** the terminal shows the shell prompt of a new pane, and typed input reaches it

### Requirement: Reconnect while attached
An attached client SHALL ping every 5 s and SHALL take 15 s without hearing anything (pongs included), a failed send or a read error as a lost connection. It SHALL then show `tilr: disconnected (<why>), reconnecting… (Ctrl+] d detaches)` in reverse video on the bottom row, drop keys typed while disconnected (except the prefix and its `d`), and connect again with the same `client_id`, 0.1 s after the loss and then with waits doubling up to 2 s.

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

## ADDED Requirements

### Requirement: Signals while attached
When an attached `tilr` gets SIGTERM, SIGHUP or SIGINT it SHALL restore the terminal (cooked mode, cursor shown, main screen, mouse capture and bracketed paste off) and then end by that signal, printing nothing; the pane goes on.

**Verified by**: `client::a_terminated_attach_restores_the_terminal`.

#### Scenario: SIGTERM while attached
- **WHEN** `kill -TERM` is sent to an attached `tilr`
- **THEN** `tilr` ends by SIGTERM, `stty -a` on its terminal shows `icanon` and `echo`, the cursor is shown, and the pane is still listed by `tilr ls`

### Requirement: Reconnect attempts
A reconnect attempt that connects but then fails to list the panes or to subscribe SHALL count as a failed attempt (status line, backoff), not end the attach. Keys SHALL be read while an attempt runs, so Ctrl+] d MUST detach at once also then.

**Verified by**: `client::an_error_right_after_a_reconnect_is_retried`, `client::detach_works_while_a_reconnect_attempt_hangs`.

#### Scenario: Error right after a reconnect
- **WHEN** a reconnect attempt connects and the connection is lost before `ListPanes` is answered
- **THEN** `tilr` stays attached, shows the error on the status line and connects again, and keys typed once it is back reach the pane

#### Scenario: Detach while an attempt hangs
- **WHEN** a reconnect attempt is waiting on a socket that accepts but never answers and the user types Ctrl+] d
- **THEN** `tilr` prints `[detached from pane …]` and exits 0 at once, not after the 5 s connect timeout (the test allows 1 s)
