## MODIFIED Requirements

### Requirement: Login links the machine through a device code
`tilrd cloud login [--url URL] [--name NAME]` (URL: `--url`, `TILR_CLOUD`, or `https://wss.tilr.dev`; NAME: the host name) SHALL POST `{server_id, name, platform, version}` to `<URL>/v1/enroll`, print the `verify_url` and the `user_code`, and poll `<URL>/v1/enroll/poll` with `{device_code}` every `interval` s (default 3, clamped to 1–30), through network errors. An answer without `device_code` MUST fail with `<URL> did not answer as tilr Cloud`.

**Verified by**: `cloud::login_prints_the_code_and_polls_until_confirmed`; unit test `cloud::tests::the_default_cloud_is_the_api_host` (crates/tilrd/src/cloud.rs) for the default; `cloud::cloud_login_help_names_the_default_api_host` for help.

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

**Verified by**: `cloud::login_keeps_the_link_once_confirmed_and_an_expired_code_links_nothing`; `cloud::a_linked_server_gets_its_tunnel_from_cloud` for a linked machine keeping its Cloud.

#### Scenario: Confirmed in the browser
- **WHEN** the user confirms the printed code at the printed link
- **THEN** `cloud.json` (mode 0600) holds the Cloud's URL, the machine id and the machine token, and the command prints `Linked. A running tilrd connects on its own.`

#### Scenario: The code expires
- **WHEN** the poll answers 404
- **THEN** the command fails with `the code expired, nothing was linked` and no `cloud.json` is written

#### Scenario: A linked machine keeps its Cloud
- **WHEN** a server starts with an existing `cloud.json` naming a custom Cloud URL
- **THEN** it connects to that Cloud and leaves `cloud.json` unchanged regardless of the login default
