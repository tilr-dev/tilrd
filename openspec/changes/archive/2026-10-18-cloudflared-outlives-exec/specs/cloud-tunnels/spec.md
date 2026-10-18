## MODIFIED Requirements

### Requirement: cloudflared does not outlive the server
`cloudflared` SHALL stop with the server and run in a process group of its own, which a signal to the server's group MUST NOT reach. After a SIGKILL of the server none MUST be left: on Linux it SHALL get SIGTERM when the server dies, and everywhere a server starting on the same runtime directory SHALL first stop the `cloudflared` named in `<runtime dir>/cloudflared/pid` if it still runs that directory's `config.yml`.

**Verified by**: `cloud::a_cloudflared_left_by_a_killed_server_is_stopped`, `cloud::cloudflared_is_not_in_the_servers_process_group`; itests `UC-088` (`uc_088.py::cloudflared_is_gone_after_the_server_is_sigkilled`).

#### Scenario: The server stops
- **WHEN** a server with its tunnel up stops (`tilr kill-server`, SIGTERM)
- **THEN** its `cloudflared` is gone

#### Scenario: SIGKILL
- **WHEN** a server with its tunnel up is killed with SIGKILL on Linux
- **THEN** within 5 s no `cloudflared` it started is left running

#### Scenario: Leftover found at the next start
- **WHEN** a `cloudflared` recorded in `<runtime dir>/cloudflared/pid` still runs with that directory's `config.yml` and a server starts on that runtime directory
- **THEN** that `cloudflared` is stopped before any new one starts and the log says `stopped a cloudflared left by an earlier server`

#### Scenario: Terminal hangup
- **WHEN** SIGHUP is sent to the process group of a server with its tunnel up
- **THEN** `cloudflared` keeps its pid and the tunnel stays `up`
