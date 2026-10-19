## MODIFIED Requirements

### Requirement: Login links the machine through a device code
`tilrd cloud login [--url URL] [--name NAME]` (URL: `--url`, `TILR_CLOUD`, or `https://cloud.tilr.dev`; NAME: the host name) SHALL POST `{server_id, name, platform, version}` to `<URL>/v1/enroll`, print the `verify_url` and the `user_code`, and poll `<URL>/v1/enroll/poll` with `{device_code}` every `interval` s (default 3, clamped to 1–30), through network errors. An answer without `device_code` MUST fail with `<URL> did not answer as tilr Cloud`.

**Verified by**: `cloud::login_prints_the_code_and_polls_until_confirmed` (planned in change cloud-tunnels-tests).

#### Scenario: The code is shown
- **WHEN** the user runs `tilr cloud login`
- **THEN** it prints `To link this machine, open`, the link, `and confirm the code <user_code>`, and waits

### Requirement: Login keeps the link once confirmed
When the poll answers `status` `approved`, login SHALL write the link file `cloud.json` in the state directory (`url`, `machine_id`, `token`; mode 0600, written aside and renamed) and print `Linked. A running tilrd connects on its own.`. A 404 from the poll MUST fail with `the code expired, nothing was linked` and write nothing.

**Verified by**: `cloud::login_keeps_the_link_once_confirmed_and_an_expired_code_links_nothing` (planned in change cloud-tunnels-tests).

#### Scenario: Confirmed in the browser
- **WHEN** the user confirms the printed code at the printed link
- **THEN** `cloud.json` (mode 0600) holds the Cloud's URL, the machine id and the machine token, and the command prints `Linked. A running tilrd connects on its own.`

#### Scenario: The code expires
- **WHEN** the poll answers 404
- **THEN** the command fails with `the code expired, nothing was linked` and no `cloud.json` is written

### Requirement: Logout unlinks the machine
`tilrd cloud logout` SHALL, without a link, print `This machine is not linked.`; with one, send `DELETE <url>/v1/machine` with `Authorization: Bearer <machine token>`, remove `cloud.json`, and print `Unlinked.` when Cloud answered (a 401 counts) or `Unlinked here, but <url> could not be told (<error>): remove the machine there.` when it could not be told. The link file MUST be removed in both cases.

**Verified by**: `cloud::logout_unlinks_the_machine_whether_or_not_cloud_hears` (planned in change cloud-tunnels-tests).

#### Scenario: Cloud unreachable
- **WHEN** a linked machine runs `tilr cloud logout` while Cloud cannot be reached
- **THEN** `cloud.json` is removed and the output says Cloud could not be told

### Requirement: Cloud may revoke the link
When the control connection's handshake is refused with HTTP 401, or Cloud closes it with code 4001, the server SHALL stop its tunnel, log `<url> does not know this machine; link it again with: tilr cloud login` (unless the link was removed locally), and MUST NOT connect again until the link file changes.

**Verified by**: `cloud::a_link_cloud_revokes_stops_the_tunnel_until_linked_again` (planned in change cloud-tunnels-tests).

#### Scenario: Machine removed in Cloud
- **WHEN** Cloud closes the control connection with code 4001
- **THEN** the tunnel is stopped and the server does not reconnect until `cloud.json` changes

### Requirement: The control connection is kept alive and retried
The server SHALL send the text message `ping` every 30 s and MUST give the connection up after 100 s in which nothing came from Cloud (a `pong` or any message counts), and reconnect after a failure with a delay starting at 1 s and doubling up to 60 s (the connect itself is given 20 s). While Cloud is away the tunnel MUST stay as it is, so clients using it are not cut off.

**Verified by**: `cloud::the_control_connection_is_retried_and_the_tunnel_kept_meanwhile` (planned in change cloud-tunnels-tests).

#### Scenario: Cloud goes away
- **WHEN** the control connection drops while a tunnel is up
- **THEN** `cloudflared` keeps running and the server retries the connection after 1 s, 2 s, 4 s … at most 60 s apart

### Requirement: A cloudflared that fails is reported and restarted
When `cloudflared` cannot be started or exits on its own while the tunnel is wanted, the server SHALL report `tunnel_status` `failed` with the reason (`start <path>: …` or `cloudflared stopped: <exit status>`) and start it again after 5 s, or at once when Cloud sends another tunnel.

**Verified by**: `cloud::a_cloudflared_that_exits_is_reported_and_started_again` (planned in change cloud-tunnels-tests).

#### Scenario: cloudflared not installed
- **WHEN** Cloud sends `tunnel_open` and the `cloudflared` path does not exist
- **THEN** Cloud receives `tunnel_status` `failed` whose detail names the path, and the server tries again 5 s later

### Requirement: No TCP port, no Cloud
A server with no TCP port (`--no-listen`, or a server `tilr` started that found its port taken) SHALL NOT connect to Cloud or run a tunnel, logging `no TCP port: no tilr Cloud tunnel`.

**Verified by**: `cloud::a_server_without_a_tcp_port_does_not_connect_to_cloud` (planned in change cloud-tunnels-tests).

#### Scenario: --no-listen
- **WHEN** a linked server runs with `--no-listen`
- **THEN** Cloud receives no control connection and no `cloudflared` runs
