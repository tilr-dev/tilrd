## MODIFIED Requirements

### Requirement: The pane process shows which pane it is
Each pane SHALL be its own process, a child of the supervisor, started as `tilrd pane --id <pane_id>` followed by `--cols N --rows N` only when the size is not 80 x 24, `--cwd <dir>` when there is one, and `-- <command...>` when a command was given. It SHALL keep these start arguments for its whole life, reloads included: a reload SHALL exec the successor with them, whatever its process title shows, and its visible command line (`ps -o args`, `/proc/<pid>/cmdline`) SHALL start with them.

**Verified by**: `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps`.

#### Scenario: Default size is left out
- **WHEN** a client creates an 80 x 24 pane and a 100 x 30 pane running `/bin/sh` in the home directory
- **THEN** their pane processes' command lines after `pane` start with `--id <id1> --cwd <home> -- /bin/sh` and `--id <id2> --cols 100 --rows 30 --cwd <home> -- /bin/sh`

#### Scenario: Start arguments survive a reload
- **WHEN** a pane started with `--cols 100 --rows 30` has shown a title and the server reloads
- **THEN** the new pane process image's command line still starts with `--id <id> --cols 100 --rows 30`, and the pane keeps its shell

## ADDED Requirements

### Requirement: The pane process title follows the pane
After its start arguments, a pane process SHALL show ` [shell <shell pid> in <cwd>]`, or ` [shell <shell pid> in <cwd>: <command>]` while another process group is in the foreground of its pty (`<cwd>` is `PaneInfo.cwd`, `<command>` the group leader's command line, at most 80 bytes). It SHALL update the title within a second of either changing and SHALL set it again after a reload.

**Verified by**: `panes::the_pane_process_title_follows_the_directory_and_the_command` and `reload::the_pane_process_title_is_set_again_after_a_reload`.

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

### Requirement: A title that does not fit is cut
A title longer than the space the pane process has for it SHALL be cut at the end of that space; when no title can be set at all, the command line SHALL stay the start arguments and the server SHALL log `pane title: no room` once.

**Verified by**: unit test `title::tests::a_title_is_cut_to_the_area` (crates/tilrd/src/title.rs).

#### Scenario: A very long directory
- **WHEN** the shell enters a directory whose path is longer than the title space
- **THEN** the command line starts with the start arguments and ends where the space ends, and the pane works as before

### Requirement: Making room for a title preserves the environment
Moving the environment to make room for the title SHALL preserve every environment entry and the pane's original arguments.

**Verified by**: unit test `title::tests::the_environment_is_unchanged_after_the_move`.

#### Scenario: A title overwrites the original environment strings
- **WHEN** the title is written over the original argv and environment string area
- **THEN** the copied environment entries and runtime arguments retain their original values
