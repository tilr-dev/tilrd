## ADDED Requirements

### Requirement: The commands the Tilr app runs keep their output
`tilrd ps --url <ws url> --server-version` SHALL print the server's `HelloAck.server_version` on one line and exit 0; `tilrd token` SHALL print the token (see `access-control`); `tilrd state-formats` SHALL print one `<name> <version>` line per format it reads. These commands MAY stay hidden from `--help`, and a change to their output is a change to the Tilr app's contract.

**Verified by**: `panes::ps_lists_the_panes_of_a_running_server` and `panes::ps_server_version_fails_when_nothing_listens` (`ps --url --server-version`), `auth::the_token_is_kept_for_the_user_only_and_across_restarts` (`token`), `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` (`state-formats`).

#### Scenario: Health check
- **WHEN** the app runs `tilrd ps --url ws://127.0.0.1:<port> --server-version` against a running server
- **THEN** it prints that server's version alone on one line and exits 0

#### Scenario: Nothing listening
- **WHEN** nothing listens at the URL
- **THEN** it exits non-zero within 5 s and prints no version
