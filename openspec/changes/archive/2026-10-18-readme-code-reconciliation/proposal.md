# Proposal: Reconcile README.md and USE_CASES.md with the code

## Why

Writing `openspec/specs/` (2026-10-18) found statements in README.md, USE_CASES.md and AGENTS.md that the code contradicts; the specs follow the code and list each in their Notes. Re-checked against 47a8557 (README, USE_CASES.md and TODO.md rewritten in English with the merge of `main`): the merged README already fixed the release platforms, the installer's service, the Rust version (1.90+), the `agent` feature, Cloud needing the TCP port and the link to `../apps/mac/PRODUCTION.md`; those items are dropped. Re-checked again at 1163abd: every item below is still so; line numbers are at 1163abd. The code review of branch `itests` (2026-10-18, "Repo rules") also named the `TCP_NOTSENT_LOWAT` value.

## Decision

- A connection closed as superseded (4000) is not reconnected, as README "Protocol" (README.md:681) and the `Hello.client_id` comment (`tilr.proto:66-75`) say; `tilr attach` is brought in line (it reconnects on any loss today: `crates/tilr/src/attach.rs:184-200`, `conn.rs:154-157` drops the close code). Reasoning: superseding is how the server tells a client instance that a newer connection of the same instance took over; `tilr` makes one `client_id` per run, so a 4000 reaching a live `tilr` means another process speaks under its id, and reconnecting would make the two take the pane from each other in a loop. tilr-driver already honours it (`crates/tilr-driver/src/session.rs:557`).
- AGENTS.md "Releases" says release-please "also maintains `version.txt`"; no `version.txt` exists and `release-please-config.json` names none (spec `release-process` Notes), but release-please's `simple` strategy may write it by default in its release PR, and none has been merged yet. Which side is wrong is settled from release-please's source for the pinned version (task 4.4), not guessed.

## What Changes

- README "Protocol" (README.md:687): `TCP_NOTSENT_LOWAT` is 4 KiB, not 16 KiB (`UNSENT_LIMIT`, `crates/tilrd/src/supervisor.rs:78`).
- README "Protocol" (README.md:675-683): superseding needs the same `client_id`, the same token and the same transport (Unix socket or TCP) (`supervisor.rs:2645-2656`).
- README "Protocol" (README.md:657-659): a server "sent anything else first answers with an `Error` and closes" — an undecodable packet before `Hello` gets `bad packet: …` and the connection stays; an unknown message type is ignored (`supervisor.rs:1995-1998`; spec `protocol` Notes). (protocol-v3 rewrites the paragraph above it; this one is fixed after.)
- README "Who may connect" (README.md:265-286): the Origin check also applies on the Unix socket (`supervisor.rs:1026`; spec `access-control` Notes).
- README "Metrics" (README.md:795): the content type has `; charset=utf-8` (`crates/tilrd/src/metrics.rs:20`); README "Running" (README.md:202-205): for metrics the 10 s is the time to send the request head, then `400` (spec `metrics` Notes).
- README "Reloading" (README.md:596): "about a second" → one to two seconds (the file must look the same on two 1 s checks; spec `reload` Notes).
- README pane environment (README.md:35-36): `HOME`, `USER`, `LOGNAME` only when the server's environment lacks them; `COLORTERM=truecolor` (spec `panes` Notes).
- README "Pasting images" (README.md:586): "Linux only" → the shims are installed on macOS too, macOS programs just do not use them (spec `clipboard-host-events` Notes).
- README "Disconnects" for `tilr` (README.md:149-154): ping and the 15 s timeout also on the Unix socket; the first reconnect attempt is 0.1 s after the loss; the wheel is the outer terminal's unless the program asks for the mouse (spec `tilr-cli` Notes); a 4000 close ends the attach (below).
- README "Starting at boot" (README.md:318-345): `tilr service install` takes the socket from `--socket`/`TILR_SOCKET` or the default, not `client.toml` (spec `tilr-cli` Notes).
- `tilr attach` ends on close code 4000 with `[superseded by another connection of this client]`, exit status 0, instead of reconnecting.
- USE_CASES.md:361-362: an old client without `host-events` is counted as a recipient of `tilr edit` — the code counts only clients with the feature (`supervisor.rs:2273`); rewritten.
- USE_CASES.md lacks `panes::a_pane_without_a_command_runs_the_users_login_shell`, `panes::the_configuration_file_names_the_allowed_origins` and `update::app_managed_server_never_contacts_the_standalone_update_feed`; added.
- itests/USE_CASES.md:168 UC-082 says a 65535x65535 `Resize` is refused — the code clamps to 1..65535 (spec `panes`); the line says so.
- The specs' Notes bullets for each item fixed here are removed when the change is archived.

## Capabilities

### Modified Capabilities
- `tilr-cli`: see `specs/tilr-cli/spec.md` (a superseded attach ends).
- Notes only (no requirement changes): `protocol`, `client-liveness`, `clipboard-host-events`, `release-process`, `panes`, `access-control`, `metrics`, `reload`.

## Non-goals

- Behavior changes besides `tilr`'s 4000 handling; a README line that turns out to describe wanted behavior the code lacks goes to its own change.
- The protocol rule and the features list in README "Protocol" (change `protocol-v3`).

## Impact

README.md, USE_CASES.md, itests/USE_CASES.md (UC-082 text), AGENTS.md or the `release-process` Note (task 4.4), `crates/tilr/src/{attach,conn}.rs` (close code 4000), `crates/tilr/tests/client.rs`, the Notes of the specs listed. Not breaking: `tilr` is the only client changed, and only for a close it should not get. Reload and update: nothing crosses a reload; `tilr` and `tilrd` from the same archive.
