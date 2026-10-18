# Proposal: Reconcile README.md and USE_CASES.md with the code

## Why

Writing `openspec/specs/` (2026-10-18) found statements in README.md, USE_CASES.md and AGENTS.md that the code contradicts; the specs follow the code and list each in their Notes. Re-checked against 47a8557 (README, USE_CASES.md and TODO.md rewritten in English with the merge of `main`): the merged README already fixed the release platforms (x86_64 and aarch64), the installer's service (it sets up none), the Rust version (1.90+), the `agent` feature (advertised only when forwarding is on), Cloud needing the TCP port (README "tilr Cloud": `--no-listen` gets no tunnel) and the link to `../apps/mac/PRODUCTION.md` (now the desktop-mac repository); those items are dropped.

## What Changes

- README "Protocol" (README.md:661): `TCP_NOTSENT_LOWAT` is 4 KiB, not 16 KiB (`supervisor.rs:78`).
- README "Protocol" (README.md:655) and `tilr.proto` (`Hello.client_id`): a client closed as superseded does not reconnect — but `tilr attach` reconnects after a 4000 close (`attach.rs`, `conn.rs:152`); decide which side changes.
- README (README.md:650): superseding needs the same `client_id` and token — the code also requires the same transport (`supervisor.rs:2499-2506`).
- README (README.md:632): a server "sent anything else first answers with an `Error` and closes" — an undecodable packet before `Hello` gets `bad packet` and an unknown message type is ignored (`supervisor.rs:1847-1852`; spec `protocol`).
- README "Who may connect": the Origin check also applies on the Unix socket (`supervisor.rs:962`; spec `access-control`).
- README "Metrics" (README.md:769): content type has `; charset=utf-8`; README "Running" (README.md:199-200): for metrics the 10 s is the time to send the request head (spec `metrics`).
- README "Reloading" (README.md:587): "about a second" — the file must look the same on two 1 s checks (spec `reload`).
- README pane environment (README.md:35-36): `HOME`, `USER`, `LOGNAME` only when the server's environment lacks them; `COLORTERM=truecolor` unsaid (spec `panes`).
- README "Pasting images" (README.md:577): "Linux only" — the shims are installed on macOS too (spec `clipboard-host-events`).
- README "Disconnects" for `tilr` (README.md:145-152): ping/timeout also on the Unix socket; first reconnect attempt 0.1 s after the loss; the wheel is not turned into `Scroll` (spec `tilr-cli` Notes).
- README "Starting at boot": `tilr service install` takes the socket from `--socket`/`TILR_SOCKET` or the default, not `client.toml` (spec `tilr-cli` Notes).
- USE_CASES.md:342-343: an old client without `host-events` is counted as a recipient — the code counts only clients with the feature.
- USE_CASES.md lacks `panes::a_pane_without_a_command_runs_the_users_login_shell`, `panes::the_configuration_file_names_the_allowed_origins` and `update::app_managed_server_never_contacts_the_standalone_update_feed`.
- itests/USE_CASES.md UC-082 says a 65535x65535 `Resize` is refused — the code clamps to 65535.
- AGENTS.md "Releases": release-please "also maintains `version.txt`" — there is no `version.txt` and `release-please-config.json` names none (spec `release-process` Notes).

## Capabilities

### Modified Capabilities
- `protocol`
- `client-liveness`
- `clipboard-host-events`
- `release-process`
- `tilr-cli`
- `panes`
- `access-control`
- `metrics`
- `reload`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Out of scope

- Behaviour changes, except where a line above says "decide"; those go to their own change if the code is to change.

## Impact

README.md, USE_CASES.md, itests/USE_CASES.md (UC-082 text), AGENTS.md, `tilr.proto` comments. Docs only: no reload, update or protocol impact.
