## ADDED Requirements

### Requirement: tilr checks who serves the socket
Before it connects to the default socket or starts a server for it, `tilr` SHALL refuse a runtime directory that is a symbolic link, is not a directory, is not owned by its effective uid or has permission bits for group or others, and connect to and start nothing. For any socket, before sending a byte, `tilr` MUST compare the peer's uid with its effective uid and, on a mismatch, close and fail with `socket <path> is served by another user (uid <N>)`.

**Verified by**: unit test `local::tests::a_runtime_directory_of_this_user_alone_is_trusted` (crates/tilr-proto/src/local.rs), `client::a_runtime_directory_open_to_others_or_a_link_is_refused_and_nothing_started`, itests `UC-03A` (`uc_03a.py::a_runtime_directory_another_user_made_in_tmp_is_not_entered`, `uc_03a.py::a_socket_another_user_serves_at_a_named_path_is_refused_before_a_byte`), `UC-034` (`uc_034.py::another_user_cannot_use_the_socket`).

#### Scenario: Another user's runtime directory
- **WHEN** `XDG_RUNTIME_DIR` is unset and another user made `/tmp/tilr-<uid>` and serves a socket in it
- **THEN** `tilr ls`, `tilr attach` and `tilr new -d` exit non-zero naming the directory and its owner's uid, nothing connects to that socket and no tilrd is started

#### Scenario: A runtime directory open to others or a link
- **WHEN** the runtime directory is mode 0755, or a symbolic link to a private directory
- **THEN** `tilr` refuses with `open to other users (mode 0755)` or `is a symbolic link` and starts no server

#### Scenario: A socket served by another user
- **WHEN** the socket path (`TILR_SOCKET`, `--socket` or `client.toml`'s `socket`) is served by a process of another uid
- **THEN** `tilr` prints `socket <path> is served by another user (uid <N>)` and exits non-zero without sending a byte (no WebSocket handshake, no `Hello`)

## MODIFIED Requirements

### Requirement: Token for a remote target
For a `connect` target the token SHALL be `--token` / `TILR_TOKEN`, else the file's `token`; when the target's host is on the loopback (`localhost`, an address in `127.0.0.0/8`, `::1`), else the `token` file in the state directory (`$TILR_STATE_DIR`, else `$XDG_STATE_HOME/tilr`, else `~/.local/state/tilr`), trimmed; else empty. The token from the state directory MUST NOT be sent to any other host, a name that resolves to the loopback included.

**Verified by**: unit tests `target::tests::the_command_line_beats_the_file`, `target::tests::the_stored_token_goes_to_loopback_targets_only` (crates/tilr/src/target.rs).

#### Scenario: Own machine's port without a token
- **WHEN** the user runs `tilr --connect ws://127.0.0.1:7890 ls` with no token given anywhere
- **THEN** the `Hello.token` is the contents of `~/.local/state/tilr/token`

#### Scenario: Remote target without its own token
- **WHEN** `tilr --connect ws://example.net:7890 ls` runs with a token in the state directory and none given for that target
- **THEN** the `Hello` sent carries an empty token

#### Scenario: Remote target with its own token
- **WHEN** `tilr --connect ws://example.net:7890 --token T ls` runs, or `client.toml` gives `token = "T"`
- **THEN** the `Hello` sent carries `T`
