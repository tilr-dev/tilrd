## MODIFIED Requirements

### Requirement: Login links the machine through a device code
`tilrd cloud login [--url URL] [--name NAME]` (URL: `--url`, `TILR_CLOUD`, or `https://wss.tilr.dev`; NAME: the host name) SHALL POST `{server_id, name, platform, version}` to `<URL>/v1/enroll`, print the `verify_url` and the `user_code`, and poll `<URL>/v1/enroll/poll` with `{device_code}` every `interval` s (default 3, clamped to 1–30), through network errors. An answer without `device_code` MUST fail with `<URL> did not answer as tilr Cloud`.

**Verified by**: unit test `cloud::tests::the_default_cloud_is_the_api_host` (crates/tilrd/src/cloud.rs) for the default; the rest has no test (see Notes).

#### Scenario: The code is shown
- **WHEN** the user runs `tilr cloud login`
- **THEN** it prints `To link this machine, open`, the link, `and confirm the code <user_code>`, and waits

#### Scenario: The default Cloud
- **WHEN** `tilrd cloud login` runs without `--url` and with `TILR_CLOUD` unset or empty
- **THEN** it posts to `https://wss.tilr.dev/v1/enroll`, and once linked its control connection goes to `wss://wss.tilr.dev/v1/machine/connect`
