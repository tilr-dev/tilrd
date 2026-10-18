## ADDED Requirements

### Requirement: Service-manager variables stay out of execs
An exec's environment SHALL NOT contain `NOTIFY_SOCKET`, `LISTEN_FDS`, `LISTEN_PID` or `WATCHDOG_USEC` unless the `ExecStart` sets them itself.

#### Scenario: Exec under a notify unit
- **WHEN** the server runs with `NOTIFY_SOCKET` set and an exec runs `env`
- **THEN** the output has no `NOTIFY_SOCKET`
