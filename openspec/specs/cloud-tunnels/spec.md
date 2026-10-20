# cloud-tunnels Specification

## Purpose

Covers the optional link to tilr Cloud: `tilrd cloud login|status|logout` and the link file, the outgoing control connection a linked server keeps, and the tunnel Cloud may ask for, carried by a `cloudflared` the server runs, writes the configuration of, and stops. It also states what Cloud cannot do (reach panes, choose where the tunnel leads). How `tilr cloud …` hands these commands to the running server is owned by the `tilr-cli` spec; the access token clients need through the tunnel by the `access-control` spec.

## Requirements

### Requirement: Login links the machine through a device code
`tilrd cloud login [--url URL] [--name NAME]` (URL: `--url`, `TILR_CLOUD`, or `https://wss.tilr.dev`; NAME: the host name) SHALL POST `{server_id, name, platform, version}` to `<URL>/v1/enroll`, print the `verify_url` and the `user_code`, and poll `<URL>/v1/enroll/poll` with `{device_code}` every `interval` s (default 3, clamped to 1–30), through network errors. An answer without `device_code` MUST fail with `<URL> did not answer as tilr Cloud`.

**Verified by**: unit test `cloud::tests::the_default_cloud_is_the_api_host` (crates/tilrd/src/cloud.rs) for the default; integration test `cloud::cloud_login_help_names_the_default_api_host` for help; the rest has no test (see Notes).

**Verified by**: `cloud::login_prints_the_code_and_polls_until_confirmed`.

#### Scenario: The code is shown
- **WHEN** the user runs `tilr cloud login`
- **THEN** it prints `To link this machine, open`, the link, `and confirm the code <user_code>`, and waits

#### Scenario: The default Cloud
- **WHEN** `tilrd cloud login` runs without `--url` and with `TILR_CLOUD` unset or empty
- **THEN** it posts to `https://wss.tilr.dev/v1/enroll`, and once linked its control connection goes to `wss://wss.tilr.dev/v1/machine/connect`

#### Scenario: Help names the default API host
- **WHEN** `tilrd cloud login --help` runs with `TILR_CLOUD` unset
- **THEN** it shows `[default: https://wss.tilr.dev]`

### Requirement: Login keeps the link once confirmed
When the poll answers `status` `approved`, login SHALL write the link file `cloud.json` in the state directory (`url`, `machine_id`, `token`; mode 0600, written aside and renamed) and print `Linked. A running tilrd connects on its own.`. A 404 from the poll MUST fail with `the code expired, nothing was linked` and write nothing.

**Verified by**: `cloud::login_keeps_the_link_once_confirmed_and_an_expired_code_links_nothing`.

#### Scenario: Confirmed in the browser
- **WHEN** the user confirms the printed code at the printed link
- **THEN** `cloud.json` (mode 0600) holds the Cloud's URL, the machine id and the machine token, and the command prints `Linked. A running tilrd connects on its own.`

#### Scenario: The code expires
- **WHEN** the poll answers 404
- **THEN** the command fails with `the code expired, nothing was linked` and no `cloud.json` is written

#### Scenario: A linked machine keeps its Cloud
- **WHEN** a server starts with an existing `cloud.json` naming a custom Cloud URL
- **THEN** it connects to that Cloud and leaves `cloud.json` unchanged regardless of the login default

### Requirement: Status says whether the machine is linked
`tilrd cloud status` SHALL print `Linked to <url> as machine <machine_id>.` when `cloud.json` is readable, else `This machine is not linked. Link it with: tilr cloud login`; it exits 0 either way. `tilr cloud status` runs it in the server.

**Verified by**: `client::server_commands_run_in_the_server`.

#### Scenario: Not linked
- **WHEN** `tilr cloud status` runs on a machine without `cloud.json`
- **THEN** the output contains `tilr cloud login`

### Requirement: Logout unlinks the machine
`tilrd cloud logout` SHALL, without a link, print `This machine is not linked.`; with one, send `DELETE <url>/v1/machine` with `Authorization: Bearer <machine token>`, remove `cloud.json`, and print `Unlinked.` when Cloud answered (a 401 counts) or `Unlinked here, but <url> could not be told (<error>): remove the machine there.` when it could not be told. The link file MUST be removed in both cases.

**Verified by**: `cloud::logout_unlinks_the_machine_whether_or_not_cloud_hears`.

#### Scenario: Cloud unreachable
- **WHEN** a linked machine runs `tilr cloud logout` while Cloud cannot be reached
- **THEN** `cloud.json` is removed and the output says Cloud could not be told

### Requirement: A linked server keeps one control connection to Cloud
A server with a TCP port and a `cloud.json` SHALL keep one WebSocket to `<url>/v1/machine/connect` (`wss` for `https`) with `Authorization: Bearer <machine token>`, first sending `{"type":"hello","protocol":1,"server_id","platform","version","auth"}` (`auth`: whether clients need the token). On `hello_ack` and every change it MUST send `{"type":"tunnel_status","state","detail"}`, `state` being `down`, `starting`, `up` or `failed`.

**Verified by**: `cloud::a_linked_server_gets_its_tunnel_from_cloud`; itests `UC-07A` (`uc_07a.py::tunnel_open_starts_cloudflared_at_this_server_and_tunnel_close_stops_it`).

#### Scenario: Connecting
- **WHEN** a server with a token starts with `cloud.json` naming a Cloud
- **THEN** that Cloud receives a connection with `Authorization: Bearer <machine token>` and a `hello` with `protocol` 1, `auth` true and the server's platform
- **AND** after `hello_ack` it receives `tunnel_status` with `state` `down`

### Requirement: A running server notices a login or a logout
The server SHALL look at the link file every 3 s: a server that was not linked MUST connect once `cloud.json` appears, and a linked one MUST close its control connection (and stop its tunnel) once the file changes or goes, connecting again with the new link if there is one.

**Verified by**: `cloud::a_server_linked_while_it_runs_connects_on_its_own`.

#### Scenario: Linked while running
- **WHEN** `state/cloud.json` is written while the server runs
- **THEN** the server connects to that Cloud

#### Scenario: Unlinked while running
- **WHEN** `state/cloud.json` is removed
- **THEN** the server closes the control connection

### Requirement: Cloud may revoke the link
When the control connection's handshake is refused with HTTP 401, or Cloud closes it with code 4001, the server SHALL stop its tunnel, log `<url> does not know this machine; link it again with: tilr cloud login` (unless the link was removed locally), and MUST NOT connect again until the link file changes.

**Verified by**: `cloud::a_link_cloud_revokes_stops_the_tunnel_until_linked_again`.

#### Scenario: Machine removed in Cloud
- **WHEN** Cloud closes the control connection with code 4001
- **THEN** the tunnel is stopped and the server does not reconnect until `cloud.json` changes

### Requirement: The control connection is kept alive and retried
The server SHALL send the text message `ping` every 30 s and MUST give the connection up after 100 s in which nothing came from Cloud (a `pong` or any message counts), and reconnect after a failure with a delay starting at 1 s and doubling up to 60 s (the connect itself is given 20 s). While Cloud is away the tunnel MUST stay as it is, so clients using it are not cut off.

**Verified by**: `cloud::the_control_connection_is_retried_and_the_tunnel_kept_meanwhile`.

#### Scenario: Cloud goes away
- **WHEN** the control connection drops while a tunnel is up
- **THEN** `cloudflared` keeps running and the server retries the connection after 1 s, 2 s, 4 s … at most 60 s apart

### Requirement: tunnel_open runs cloudflared aimed at this server's port
On `{"type":"tunnel_open","tunnel":{id, account_tag, secret, hostname}}`, a server with authentication SHALL start `<cloudflared> tunnel --config <config.yml> run` as its own child (`--cloudflared PATH` / `TILR_CLOUDFLARED`, default `cloudflared`) and report `starting`, then `up` once `cloudflared` logs `Registered tunnel connection`. The same tunnel again changes nothing; another one replaces it.

**Verified by**: `cloud::a_linked_server_gets_its_tunnel_from_cloud`; itests `UC-07A` (`uc_07a.py::tunnel_open_starts_cloudflared_at_this_server_and_tunnel_close_stops_it`).

#### Scenario: Cloud opens the tunnel
- **WHEN** Cloud sends `tunnel_open` for hostname `abc234.tilr.test`
- **THEN** a `cloudflared` whose parent is the server runs as `tunnel --config FILE run` and Cloud receives `tunnel_status` `up`

### Requirement: The server writes the tunnel's configuration
For a tunnel, the server SHALL write in `cloudflared/` of the runtime directory (directory 0700, files 0600) `credentials.json` (`AccountTag`, `TunnelID`, `TunnelSecret`) and `config.yml` (`tunnel: <id>`, `credentials-file`, `no-autoupdate: true`, an `ingress:` rule for `<hostname>` to `http://<listen address>`, then `http_status:404`). An address on every interface MUST be written as `127.0.0.1`.

**Verified by**: `cloud::a_linked_server_gets_its_tunnel_from_cloud`; itests `UC-07A` (`uc_07a.py::tunnel_open_starts_cloudflared_at_this_server_and_tunnel_close_stops_it`).

#### Scenario: The written configuration
- **WHEN** a server listening on `127.0.0.1:<port>` is sent `tunnel_open` for hostname `abc234.tilr.test`
- **THEN** the config holds `ingress:\n  - hostname: abc234.tilr.test\n    service: http://127.0.0.1:<port>\n  - service: http_status:404\n` and `credentials.json` holds the secret with mode 0600

### Requirement: Cloud cannot point the tunnel elsewhere
Where the tunnel leads SHALL be the server's choice: of `tunnel` only `id` (hex, `-`), `account_tag` (hex), `secret` (letters, digits, `+/=`) and `hostname` (lower-case letters, digits, `.-`) are used, each 1–253 characters; other fields (`service`, `local`) are ignored. A tunnel breaking these rules MUST get `tunnel_status` `failed`, detail `bad tunnel <field>`, and no `cloudflared`.

**Verified by**: `cloud::a_tunnel_that_would_lead_elsewhere_is_refused`; itests `UC-07A` (`uc_07a.py::nothing_but_tunnel_commands_from_cloud_is_acted_on`).

#### Scenario: A hostname that would rewrite the config
- **WHEN** Cloud sends a tunnel with hostname `a.dev\n  - service: ssh://localhost:22`
- **THEN** the server answers `tunnel_status` `failed` with a detail naming `hostname`, and no `cloudflared` runs

#### Scenario: A service Cloud names
- **WHEN** Cloud sends a valid tunnel with extra fields `service: http://127.0.0.1:22` and `local: 127.0.0.1:22`
- **THEN** the tunnel comes up with `service: http://127.0.0.1:<the server's port>` and the config has no `:22`

### Requirement: tunnel_close stops cloudflared and removes its secret
On `tunnel_close` (and when the link goes), the server SHALL send `cloudflared` SIGTERM, kill it if it has not exited within 3 s, remove the runtime `cloudflared/` directory with the credentials, and report `tunnel_status` `down`. A later `tunnel_open` MUST start a new `cloudflared`.

**Verified by**: `cloud::a_linked_server_gets_its_tunnel_from_cloud` and itests `UC-07A` (`uc_07a.py::tunnel_open_starts_cloudflared_at_this_server_and_tunnel_close_stops_it`).

#### Scenario: Cloud closes the tunnel
- **WHEN** Cloud sends `tunnel_close` while the tunnel is up
- **THEN** Cloud receives `tunnel_status` `down`, `cloudflared` ended on SIGTERM within 5 s, and the `cloudflared/` directory is gone
- **AND** the next `tunnel_open` runs a `cloudflared` with a new pid

### Requirement: A cloudflared that fails is reported and restarted
When `cloudflared` cannot be started or exits on its own while the tunnel is wanted, the server SHALL report `tunnel_status` `failed` with the reason (`start <path>: …` or `cloudflared stopped: <exit status>`) and start it again after 5 s, or at once when Cloud sends another tunnel.

**Verified by**: `cloud::a_cloudflared_that_exits_is_reported_and_started_again`.

#### Scenario: cloudflared not installed
- **WHEN** Cloud sends `tunnel_open` and the `cloudflared` path does not exist
- **THEN** Cloud receives `tunnel_status` `failed` whose detail names the path, and the server tries again 5 s later

### Requirement: A server without authentication takes no tunnel
A server started with `--no-auth` SHALL say `"auth": false` in its `hello` and MUST answer every `tunnel_open` with `tunnel_status` `failed`, detail `the server runs without authentication`, starting no `cloudflared`.

**Verified by**: `cloud::a_server_without_authentication_takes_no_tunnel` and itests `UC-07A` (`uc_07a.py::a_server_without_authentication_gets_no_tunnel`).

#### Scenario: --no-auth
- **WHEN** a linked server runs with `--no-auth` and Cloud sends `tunnel_open`
- **THEN** Cloud receives `tunnel_status` `failed` with detail `the server runs without authentication` and no `cloudflared` is started

### Requirement: Nothing but tunnel commands from Cloud is acted on
The server SHALL act only on `hello_ack`, `tunnel_open`, `tunnel_close` and `error` (logged) from Cloud and MUST ignore any other message, keeping the connection open: nothing from Cloud reaches a pane, runs a program or writes a file but the tunnel's. It MUST send Cloud only `hello`, `tunnel_status` and `ping`: never the access token or anything of the panes.

**Verified by**: itests `UC-07A` (`uc_07a.py::nothing_but_tunnel_commands_from_cloud_is_acted_on`).

#### Scenario: Hostile messages
- **WHEN** Cloud sends `create_pane`, `input`, `paste`, `close_pane`, `exec`, `fs_write`, `clipboard_put`, `update` and `token` messages
- **THEN** no file is touched, no pane is made, closed or typed into, the server runs on, the control connection stays open, and Cloud has received only `hello` and `tunnel_status`

### Requirement: No TCP port, no Cloud
A server with no TCP port (`--no-listen`, or a server `tilr` started that found its port taken) SHALL NOT connect to Cloud or run a tunnel, logging `no TCP port: no tilr Cloud tunnel`.

**Verified by**: `cloud::a_server_without_a_tcp_port_does_not_connect_to_cloud`.

#### Scenario: --no-listen
- **WHEN** a linked server runs with `--no-listen`
- **THEN** Cloud receives no control connection and no `cloudflared` runs

### Requirement: A reload replaces cloudflared with one of the new server
Before a reload replaces the executable, the server SHALL stop its `cloudflared` (as on `tunnel_close`, without reporting `down`) and wait for it to exit, so that none runs on as a child the successor does not know; the successor connects to Cloud again and starts its own `cloudflared` on Cloud's next `tunnel_open`. If the reload fails, the server MUST start its `cloudflared` again.

**Verified by**: `cloud::a_reload_replaces_cloudflared_with_one_of_the_new_server`.

#### Scenario: SIGUSR1 with the tunnel up
- **WHEN** a server with its tunnel up gets SIGUSR1 and Cloud sends `tunnel_open` to the server that took over
- **THEN** a `cloudflared` with a new pid runs and the old one is gone

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

## Notes

- The integration tests play Cloud and `cloudflared`. A real Cloudflare tunnel has not been tested.
- The unit tests in `crates/tilrd/src/cloud.rs` (`a_tunnel_that_would_rewrite_the_config_is_refused`, `the_control_address_follows_the_cloud_address`) also cover the field checks and the control URL.
- "cloudflared does not outlive the server": the terminal-hangup scenario is covered by checking that `cloudflared` leads a process group of its own (`cloud::cloudflared_is_not_in_the_servers_process_group`); no test sends SIGHUP to a server's group, which in the test harness is the test runner's. The SIGKILL scenario on macOS (no parent-death signal) is covered by the sweep at the next start only.
- No itests case cited here is currently red.
