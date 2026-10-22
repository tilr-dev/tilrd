## MODIFIED Requirements

### Requirement: Reconnect while attached
An attached client SHALL detect lost connections and reconnect with the same `client_id`, retaining admitted input until acknowledged. It SHALL reconcile `HelloAck` counters and replay unacknowledged input in order. The delivery queue SHALL hold at most 4 MiB or 4096 messages; overflow SHALL visibly reject new input without discarding admitted input or advancing its sequence.

**Verified by**: `client::attach_replays_a_key_when_the_connection_ends_without_an_ack`, `client::attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches`, itests `UC-014` (`uc_014.py::after_a_sigkill_the_next_tilr_replaces_the_socket_and_starts_a_new_daemon`).

#### Scenario: Connection loss and retry
- **WHEN** no message (including pongs) arrives for 15 s, a send fails, or a read fails
- **THEN** the client shows `tilr: disconnected (<why>), reconnecting… (Ctrl+] d detaches)` in reverse video on the bottom row and retries after 0.1 s, doubling waits up to 2 s
- **AND** it retains keys typed while disconnected except local prefix commands; a 4000 close ends the attach
- **AND** while connected it pings every 5 s

#### Scenario: Delivery queue overflow
- **WHEN** a key would exceed the delivery queue limit
- **THEN** the client displays `input buffer full`, preserves all admitted input, and does not advance the rejected input's sequence

#### Scenario: Reload of the server
- **WHEN** the server reloads while `tilr` is attached
- **THEN** `tilr` reconnects, checks `ListPanes` for its pane, subscribes again, and admitted input typed before, during and after the disconnection reaches the pane without loss and in order; replay may duplicate input applied but not acknowledged before reload

#### Scenario: SIGKILLed server
- **WHEN** the local tilrd is killed with SIGKILL, so its socket refuses and its lock is free
- **THEN** the attached `tilr` prints `[tilrd stopped]` and exits 0

#### Scenario: Failed attempt
- **WHEN** a reconnect attempt fails for another reason
- **THEN** the status line shows the error and the attempt number, and the next attempt waits twice as long, at most 2 s
