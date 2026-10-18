## ADDED Requirements

### Requirement: Service-manager variables stay out of panes
A pane's environment SHALL NOT contain `NOTIFY_SOCKET`, `LISTEN_FDS`, `LISTEN_PID` or `WATCHDOG_USEC`, even when the server was started with them.

#### Scenario: Server under a notify unit
- **WHEN** the server runs with `NOTIFY_SOCKET` set and a pane runs `env`
- **THEN** the output has no `NOTIFY_SOCKET`
