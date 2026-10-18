## ADDED Requirements

### Requirement: The pane process title follows the pane
A pane process SHALL show in its process title the pane id, the shell's pid, the current directory and the foreground command, and SHALL update them when they change, also after a reload.

#### Scenario: cd in a pane
- **WHEN** the shell in a pane runs `cd /tmp`
- **THEN** `ps -o command -p <pane process pid>` shows `/tmp` within a second
