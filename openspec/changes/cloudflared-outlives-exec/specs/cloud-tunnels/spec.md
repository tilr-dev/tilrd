## ADDED Requirements

### Requirement: A stuck cloudflared does not hold back a reload
When `cloudflared` does not exit within its stop wait during a reload, the server SHALL kill it and MUST go on with the exec; no `cloudflared` of the old image SHALL be left running.

#### Scenario: cloudflared ignores SIGTERM
- **WHEN** a reload starts while the tunnel's `cloudflared` ignores SIGTERM
- **THEN** the reload completes and only the successor's `cloudflared` runs

### Requirement: cloudflared is not in the server's process group
`cloudflared` SHALL run in a process group of its own, so a signal to the server's group MUST NOT reach it.

#### Scenario: Terminal hangup
- **WHEN** a server started from a terminal with its tunnel up sees the terminal hang up
- **THEN** `cloudflared` keeps its pid
