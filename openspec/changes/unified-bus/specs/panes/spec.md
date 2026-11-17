## MODIFIED Requirements

### Requirement: Creating a pane
On `pane.create` the server SHALL start a pane of `cols` x `rows` cells (each clamped to 1..65535) running `command` (or, with a non-empty `profile`, what the `pane-profiles` spec says the profile and the request give together) and, once the program runs, SHALL answer the call with the pane's `PaneInfo`, then emit `pane.created`. `PaneInfo.profile` SHALL be the profile's name, or empty for a pane created without one; `PaneInfo.created_by` SHALL be the caller's identity.

**Verified by**: `panes::a_pane_runs_commands_and_reports_how_its_shell_exited`, `panes::a_pane_without_a_command_runs_the_users_login_shell` and `profiles::a_pane_starts_with_the_profile_from_the_config_directory`.

#### Scenario: A pane is created and answered by request id
- **WHEN** the owner calls `pane.create {cols: 100, rows: 30, command: []}` with call id 6
- **THEN** the reply has id 6 and is a `PaneInfo` whose `pane_id` is a UUID, whose `profile` is empty and whose `created_by` is `owner`
- **AND** `pane.list` then lists that pane with `cols` 100 and `rows` 30

#### Scenario: Commands run in the pane
- **WHEN** a client inputs `echo "sum is $((6 * 7))"` and Enter into a new pane whose `pane.output` it reads
- **THEN** its frames show `sum is 42`

#### Scenario: A pane with a profile
- **WHEN** the owner calls `pane.create {profile: "demo"}` and `demo.toml` is valid
- **THEN** the reply's `PaneInfo.profile` is `demo`

### Requirement: The directory a pane starts in
A pane SHALL start in `pane.create.cwd` when it names an existing directory on the server's machine; otherwise, for a pane created with a profile, in the profile's `cwd` when that names an existing directory; otherwise in the user's home directory. `PaneInfo.cwd` SHALL be that directory.

**Verified by**: `panes::a_pane_starts_in_the_home_directory_unless_given_one_that_exists` and `profiles::request_fields_override_the_profile`.

#### Scenario: A directory from another machine
- **WHEN** a client creates a pane with `cwd` `/no/such/directory`
- **THEN** the shell runs in the home directory and `PaneInfo.cwd` is the home directory

#### Scenario: An existing directory
- **WHEN** a client creates a pane with `cwd` naming an existing directory
- **THEN** the shell runs there and `PaneInfo.cwd` is that directory

#### Scenario: The profile's directory
- **WHEN** a client creates a pane with `cwd` `/no/such/directory` and a profile whose `cwd` exists
- **THEN** the shell runs in the profile's `cwd`
