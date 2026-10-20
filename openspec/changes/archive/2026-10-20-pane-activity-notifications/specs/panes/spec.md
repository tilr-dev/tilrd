## ADDED Requirements

### Requirement: A bell in a pane is reported
When a pane's program rings the bell (BEL in its output, outside an escape sequence), the server SHALL send `PaneActivity { pane_id, kind: BELL, at_ms }` to every greeted client, whether or not it watches the pane, and at most one per pane per second.

**Verified by**: `panes::a_bell_in_an_unwatched_pane_is_reported`, `panes::bells_are_reported_once_a_second` and `panes::a_bel_ending_an_osc_is_not_reported`.

#### Scenario: Bell in a pane nobody watches
- **WHEN** a client is greeted but not subscribed to a pane, and `printf '\a'` runs in that pane
- **THEN** the client receives `PaneActivity` with that pane's id and `kind` `BELL`

#### Scenario: Many bells
- **WHEN** a pane rings the bell ten times within 200 ms
- **THEN** a client receives one `BELL` for that pane in that second

#### Scenario: BEL ending an OSC
- **WHEN** a program sets the title with `ESC ] 2 ; t BEL`
- **THEN** no `PaneActivity` is sent

### Requirement: A finished foreground command is reported
When the pty's foreground process group of a pane goes from another group back to the shell's, the server SHALL send `PaneActivity { pane_id, kind: COMMAND_FINISHED, at_ms }` to every greeted client, once per such return; this SHALL also hold for a command that was running across a reload. A pane whose program does not use job control (a command run directly, a shell without job control) MAY report none.

**Verified by**: `panes::a_finished_foreground_command_is_reported`, `panes::typing_at_the_prompt_reports_nothing` and `reload::a_command_running_across_a_reload_is_reported_when_it_ends`.

#### Scenario: A command ends
- **WHEN** `sleep 1` runs in the foreground of a pane's shell and ends
- **THEN** every greeted client receives `PaneActivity` with that pane's id and `kind` `COMMAND_FINISHED`

#### Scenario: Typing at the prompt
- **WHEN** a user types at the shell's prompt without running a command
- **THEN** no `COMMAND_FINISHED` is sent

#### Scenario: Across a reload
- **WHEN** `sleep 3` runs in the foreground, the server reloads, and the client reconnects
- **THEN** the client receives `COMMAND_FINISHED` for that pane when `sleep` ends
