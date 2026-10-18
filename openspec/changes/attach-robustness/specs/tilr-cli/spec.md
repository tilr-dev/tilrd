## ADDED Requirements

### Requirement: Detach works while connecting
Ctrl+] d SHALL detach at once, also while `tilr attach` is connecting or reconnecting.

#### Scenario: Server unreachable
- **WHEN** `tilr attach` is reconnecting and the user types Ctrl+] d
- **THEN** `tilr` exits within 100 ms

### Requirement: The terminal is restored on a signal
When `tilr attach` is ended by SIGTERM or SIGHUP it SHALL restore the terminal settings it changed before exiting.

#### Scenario: SIGTERM while attached
- **WHEN** `kill -TERM` is sent to an attached `tilr`
- **THEN** the terminal is back in cooked mode with the cursor shown
