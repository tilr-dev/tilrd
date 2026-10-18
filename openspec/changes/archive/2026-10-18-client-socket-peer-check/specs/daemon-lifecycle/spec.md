## ADDED Requirements

### Requirement: Lock and log files are not opened through links
The server and `tilr` SHALL open the socket's lock file (`<socket>.lock`) and `tilr` the server's log (`tilrd.log`) without following a symbolic link (`O_NOFOLLOW`), and SHALL refuse one that is not a regular file of the user's; the lock file MUST NOT be truncated before the lock is held.

**Verified by**: `client::a_symlinked_lock_file_is_refused` (crates/tilr/tests/client.rs, the lock and the log), `local::a_symlinked_lock_file_is_refused` (crates/tilrd/tests/local.rs).

#### Scenario: Symlinked lock file
- **WHEN** `tilrd.sock.lock` is a symbolic link to another file
- **THEN** `tilr new -d` and `tilrd` fail with an error saying a symbolic link is not followed, no server serves the socket and the link target is unchanged

#### Scenario: Symlinked log
- **WHEN** `tilrd.log` is a symbolic link to another file and `tilr` starts a server
- **THEN** `tilr` fails with that error and the link target is unchanged
