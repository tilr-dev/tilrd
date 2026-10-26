## ADDED Requirements

### Requirement: Another server starting leaves the pane tools in place
The links in `bin/` of the runtime directory are shared by all servers of the user. A starting server SHALL leave a link that already points at its executable as it is and replace any other by a rename, so a pane of another server MUST never find `tilr`, `xclip` or `wl-paste` missing while it starts.

**Verified by**: `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools` and unit test `clipboard::tests::the_shims_are_never_missing_while_another_server_installs_them` (crates/tilrd/src/clipboard.rs).

#### Scenario: A second server starts three times
- **WHEN** a pane of one server checks `[ -e "$TILR_SHIMS/tilr" ]` in a loop while a second server of the same user starts and stops three times
- **THEN** the check never fails, and `tilr edit` in the pane still reaches its client

#### Scenario: Links replaced over and over
- **WHEN** the links are installed 2000 times, for two executables in turn, while another thread looks at `bin/tilr`
- **THEN** the link is never missing, and `bin/` holds only the links

### Requirement: A probe of the host socket is not a refused connection
A starting server SHALL tell a live server at `<server_id>.sock` by connecting to it, and the live server SHALL close a connection whose peer has gone before its uid check without logging `host socket: refused a connection`.

**Verified by**: `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools`.

#### Scenario: A second server's probe
- **WHEN** a second server of the same user starts, finds the first one's `<server_id>.sock` live and takes `<server_id>-<pid>.sock`
- **THEN** the first server's log has no `refused a connection` line
