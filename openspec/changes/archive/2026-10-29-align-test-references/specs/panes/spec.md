## MODIFIED Requirements

### Requirement: The pane process title follows the pane
After its start arguments, a pane process SHALL show ` [shell <shell pid> in <cwd>]`, or ` [shell <shell pid> in <cwd>: <command>]` while another process group is in the foreground of its pty (`<cwd>` is `PaneInfo.cwd`, `<command>` the group leader's command line, its process name when the command line cannot be read, or `?` when the leader is gone; at most 80 bytes). It SHALL update the title within a second of either changing and SHALL set it again after a reload.

**Verified by**: `panes::the_pane_process_title_follows_the_directory_and_the_command`, `panes::the_pane_process_title_follows_exec_in_the_same_foreground_group` and `reload::the_pane_process_title_is_set_again_after_a_reload`; unit tests `title::tests::a_leader_that_is_gone_is_shown_as_a_placeholder` (Linux and macOS), `title::tests::a_zombie_leader_is_named_by_its_process` (Linux), and `title::tests::another_users_leader_is_named_by_its_process` (macOS), all in crates/tilrd/src/title.rs.

#### Scenario: cd in a pane
- **WHEN** the shell in a pane runs `cd /tmp`
- **THEN** within a second `ps -o args= -p <pane process pid>` ends with `[shell <shell pid> in /tmp]`

#### Scenario: A command in the foreground
- **WHEN** the shell runs `sleep 30` in the foreground
- **THEN** within a second the title ends with `[shell <shell pid> in <cwd>: sleep 30]`
- **AND** after `sleep` is interrupted with Ctrl+C the title ends with `[shell <shell pid> in <cwd>]` again

#### Scenario: After a reload
- **WHEN** the shell is in `/tmp` and the server reloads
- **THEN** the new pane process image's title ends with `[shell <same shell pid> in /tmp]`

#### Scenario: The foreground leader execs another command
- **WHEN** a foreground wrapper waits on a test-controlled FIFO and then execs `sleep 30` without changing its pid or process group
- **THEN** the title first shows the wrapper's command line and within a second of exec shows `sleep 30`, even without pty output
