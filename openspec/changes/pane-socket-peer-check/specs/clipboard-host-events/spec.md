## ADDED Requirements

### Requirement: tilr edit and tilr open talk only to a host socket of the user's
`tilr edit` and `tilr open`, run as the pane's `tilr`, as `tilrd edit` / `tilrd open` or as the `tilr` client, SHALL read the peer credentials of `$TILR_SOCK` after connecting (`SO_PEERCRED` on Linux, `getpeereid` on macOS) and MUST NOT write a byte to a socket whose peer uid is not their own euid: they SHALL exit 1 with `tilr: cannot reach the tilr server: socket <path> is served by another user (uid N), not by this one (uid M): nothing was sent`.

**Verified by**: unit test `local::tests::a_peer_of_another_uid_is_refused` (crates/tilr-proto/src/local.rs), itests `UC-03A` (`uc_03a.py::tilr_edit_says_nothing_to_a_host_socket_another_user_serves`).

#### Scenario: Another user's socket at TILR_SOCK
- **WHEN** another uid serves a socket (mode 0666, in a directory of mode 0755) and the user runs `tilr edit notes.md`, `tilr open https://example.com` and `tilrd edit notes.md` with `TILR_PANE` set and `TILR_SOCK` naming that socket
- **THEN** each exits 1 with stderr containing `is served by another user (uid N)`, and the other user's listener received 0 bytes on every connection

#### Scenario: The user's own server
- **WHEN** `tilr edit notes.md` runs in a pane of the user's server with a listening client watching it
- **THEN** it exits 0 and the client receives the `HostEvent`
