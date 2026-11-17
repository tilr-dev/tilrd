## ADDED Requirements

### Requirement: A profile is a file named by its profile name
A profile named `<name>` SHALL be the file `profiles/<name>.toml` in the directory of the server's configuration file (`$TILR_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tilr`, else `~/.config/tilr`; with `--config PATH` or `TILR_CONFIG`, the directory of `PATH`). A missing `profiles` directory SHALL mean the server has no profiles.

**Verified by**: `profiles::a_pane_starts_with_the_profile_from_the_config_directory`.

#### Scenario: Profile in TILR_CONFIG_DIR
- **WHEN** the server runs with `TILR_CONFIG_DIR=<dir>`, `<dir>/profiles/demo.toml` says `command = ["/bin/sh"]`, and a client sends `CreatePane{profile: "demo"}`
- **THEN** it receives `PaneCreated` whose `PaneInfo.profile` is `demo` and whose `command` is `["/bin/sh"]`

### Requirement: Profile files are the user's alone
A profile file MAY be a symlink and MUST resolve to a regular file. It and the `profiles` directory MUST be owned by the server's user and MUST NOT be writable by group or others; otherwise the profile SHALL be invalid.

**Verified by**: `profiles::a_profile_others_can_write_is_refused`.

#### Scenario: Writable by others
- **WHEN** `profiles/demo.toml` has mode 0666 and a client sends `CreatePane{profile: "demo"}`
- **THEN** it receives an `Error` starting with `create pane: profile demo: ` that names the file and says it is writable by others, and no pane is listed

### Requirement: A client names a profile, never a path
`CreatePane.profile` and every profile name SHALL match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; an empty `profile` SHALL mean no profile. Any other value MUST be refused with `Error{message: "create pane: bad profile name: <name>"}` before the server touches the file system, and no pane SHALL start. A valid name without a file SHALL be refused with `Error{message: "create pane: no such profile: <name>"}`. The connection SHALL stay open in both cases.

**Verified by**: `profiles::a_profile_name_is_never_a_path`, `profiles::an_unknown_profile_is_an_error`.

#### Scenario: Traversal
- **WHEN** a client sends `CreatePane` with `profile` `../server`, `/etc/passwd`, `.hidden` or `a/b`
- **THEN** each is answered `Error` `create pane: bad profile name: <that value>` and no pane is listed

#### Scenario: Unknown name
- **WHEN** no `profiles/nope.toml` exists and a client sends `CreatePane{profile: "nope"}`
- **THEN** it receives `Error` `create pane: no such profile: nope` and can go on to create a pane without a profile

### Requirement: Profiles are parsed strictly
A profile file SHALL be TOML with only the keys `description`, `command` (strings), `cwd`, `[env]` (string values) and `[sandbox]` with `read_only`, `read_write`, `executables`, `env` (strings) and `network` (`none` default, `client`, `server`). An unknown key at any level or a value of the wrong type SHALL make the profile invalid; a pane from an invalid profile MUST be refused with an `Error` starting with `create pane: profile <name>: ` naming the file and the reason.

**Verified by**: `profiles::a_misspelt_key_makes_the_profile_invalid`, unit test `profile::tests::every_sandbox_key_is_parsed` (crates/tilrd/src/profile.rs).

#### Scenario: Misspelt key
- **WHEN** `profiles/demo.toml` contains `comand = ["/bin/sh"]` and a client creates a pane with it
- **THEN** it receives an `Error` starting with `create pane: profile demo: ` naming the file and `comand`, and no pane is listed

#### Scenario: Unknown network value
- **WHEN** a profile says `[sandbox] network = "all"`
- **THEN** the profile is invalid and the reason names `network`

### Requirement: Paths and variable names in a profile
`cwd` and every path in `[sandbox]` MUST be absolute or start with `~` (the user's home from the password database); `.` and `..` components SHALL be folded lexically. A relative path, or an `[env]` name that is empty, contains `=` or starts with `TILR_`, SHALL make the profile invalid.

**Verified by**: unit tests `profile::tests::relative_paths_are_refused`, `profile::tests::tilde_is_the_home_directory`, `profile::tests::tilr_variables_cannot_be_set` (crates/tilrd/src/profile.rs).

#### Scenario: Relative path
- **WHEN** a profile says `cwd = "work"`
- **THEN** creating a pane with it is refused, naming `cwd` and saying the path must be absolute or start with `~`

#### Scenario: Tilde
- **WHEN** a profile says `cwd = "~/proj"` and `<home>/proj` exists
- **THEN** the pane's `PaneInfo.cwd` is `<home>/proj`

#### Scenario: A server variable
- **WHEN** a profile says `[env] TILR_PANE = "x"`
- **THEN** the profile is invalid and the reason names `TILR_PANE`

### Requirement: A profile is read on every create
The server SHALL read and validate the profile file on every `CreatePane` that names it and SHALL NOT cache it; a change to the file SHALL apply to the next pane created with it and SHALL NOT change a pane that already runs.

**Verified by**: `profiles::an_edited_profile_applies_to_the_next_pane_only`.

#### Scenario: Edit between two panes
- **WHEN** a pane is created with profile `demo` (`[env] X = "1"`), the file is changed to `X = "2"`, and a second pane is created with it
- **THEN** `echo $X` prints `1` in the first pane and `2` in the second

### Requirement: Command and directory of a profile pane
A pane created with a profile SHALL run the request's `command` if it is not empty, else the profile's `command`, else the login shell. It SHALL start in the request's `cwd` if that is an existing directory, else the profile's `cwd` if that is an existing directory, else the home directory.

**Verified by**: `profiles::request_fields_override_the_profile`, `profiles::a_pane_starts_with_the_profile_from_the_config_directory`.

#### Scenario: Profile defaults
- **WHEN** a profile has `command = ["/bin/sh"]` and `cwd = "<dir>"`, and a client sends `CreatePane{profile, command: [], cwd: ""}`
- **THEN** `/bin/sh` runs in `<dir>`

#### Scenario: Request wins
- **WHEN** the same profile is used with `command` `["/bin/bash", "--norc"]` and `cwd` naming another existing directory
- **THEN** `/bin/bash --norc` runs in that directory and `PaneInfo.profile` names the profile

### Requirement: Variables of a profile pane
A pane created with a profile SHALL get the server's environment with the profile's `[env]` variables set over it (a value starting with `~/` expanded to the home directory), and then the terminal variables `TERM`, `COLORTERM`, `SHELL` and `TILR_PANE` as for any pane. No request field SHALL change `[env]` or `[sandbox]`.

**Verified by**: `profiles::request_fields_override_the_profile`.

#### Scenario: Profile variable
- **WHEN** a profile has `[env] GREETING = "hi"` and a pane is created with it, with or without an explicit `command`
- **THEN** `echo $GREETING` in the pane prints `hi`

#### Scenario: TERM stays the server's
- **WHEN** a profile sets `[env] TERM = "dumb"`
- **THEN** the pane's `TERM` is `xterm-256color`

### Requirement: A sandboxed profile is refused where it cannot be enforced
A profile with a `[sandbox]` table SHALL be validated like any profile, and creating a pane with it MUST be refused with `Error{message: "create pane: profile <name> asks for a sandbox, which this server cannot enforce"}` on a server that does not list `sandbox` in `HelloAck.features`; the pane's program MUST NOT be started, restricted or not.

**Verified by**: `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`.

#### Scenario: No sandbox support
- **WHEN** a server without the `sandbox` feature gets `CreatePane` with a profile that has `[sandbox] read_write = ["~/w"]` and `command = ["/bin/sh", "-c", "touch <marker>"]`
- **THEN** the client receives that `Error`, no pane is listed, and `<marker>` does not exist

### Requirement: Listing profiles
`ListProfiles{request_id}` SHALL be answered with `ProfileList{profiles, request_id}` listing, sorted by name, every `profiles/*.toml` whose stem is a valid profile name: `name`, `description`, `sandboxed` (the file has a `[sandbox]` table) and `error` (empty when the profile is valid, else the reason `CreatePane` would give). Files with other names SHALL be left out. A server without a `profiles` directory SHALL answer an empty list.

**Verified by**: `profiles::profiles_are_listed_with_their_errors`.

#### Scenario: Three files
- **WHEN** `profiles/` holds `a.toml` (valid), `b.toml` (with `[sandbox]`), `c.toml` (bad TOML) and `.x.toml`
- **THEN** `ProfileList` lists `a`, `b`, `c` in that order, `b` with `sandboxed` true, `c` with a non-empty `error`, and no `.x`

#### Scenario: No directory
- **WHEN** the config directory has no `profiles` directory and a client sends `ListProfiles{request_id: 4}`
- **THEN** it receives `ProfileList{profiles: [], request_id: 4}`

### Requirement: A pane keeps its profile across a reload
A pane's `PaneInfo.profile` and the command, directory, variables and sandbox it was started with SHALL be handed to the successor on a reload and SHALL NOT be read again from the profile file.

**Verified by**: `reload::a_pane_keeps_its_profile_across_a_reload`.

#### Scenario: Profile removed before a reload
- **WHEN** a pane was created with profile `demo`, `profiles/demo.toml` is deleted, and the server reloads
- **THEN** after reconnecting `ListPanes` shows the pane with `profile` `demo`, and its shell still has the profile's `[env]` variables

### Requirement: tilr profiles lists the server's profiles
`tilr profiles` SHALL send `ListProfiles` to its target and print one line per profile: the name, `sandbox` or `-`, and the description, or `error: <reason>` for an invalid profile; it SHALL exit 0 when the list was received, also when it is empty.

**Verified by**: `client::tilr_profiles_lists_the_profiles_of_the_server`.

#### Scenario: One valid, one invalid
- **WHEN** the server has a valid `demo` with `description = "a demo"` and an invalid `bad`
- **THEN** `tilr profiles` prints a line with `bad` and `error: `, then a line `demo  -  a demo`, and exits 0
