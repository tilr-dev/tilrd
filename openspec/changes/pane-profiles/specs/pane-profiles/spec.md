## ADDED Requirements

### Requirement: Profiles live in profiles_dir
A profile named `<name>` SHALL be the file `<name>.toml` in the directory `profiles_dir` of `server.toml` (absolute or starting with `~`), by default `profiles/` in the directory of the server's configuration file (`$TILR_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tilr`, else `~/.config/tilr`; with `--config PATH` or `TILR_CONFIG`, the directory of `PATH`). A missing directory SHALL mean the server has no profiles.

**Verified by**: `profiles::a_pane_starts_with_the_profile_from_the_config_directory`, `profiles::profiles_dir_names_another_directory`.

#### Scenario: Default directory
- **WHEN** the server runs with `TILR_CONFIG_DIR=<dir>`, `<dir>/profiles/demo.toml` says `command = ["/bin/sh"]`, and the owner calls `pane.create {profile: "demo"}`
- **THEN** the reply's `PaneInfo.profile` is `demo` and its `command` is `["/bin/sh"]`

#### Scenario: Own directory
- **WHEN** `server.toml` says `profiles_dir = "<other>"` and `<other>/demo.toml` exists, and `<dir>/profiles/demo.toml` does not
- **THEN** `pane.create {profile: "demo"}` starts the pane from `<other>/demo.toml`

### Requirement: Profile files are the user's alone
A profile file MAY be a symlink and MUST resolve to a regular file. It and the profiles directory MUST be owned by the server's user and MUST NOT be writable by group or others; otherwise the profile SHALL NOT load, and `pane.create` and `pane.profiles` SHALL report the file and the reason (`is writable by others`, `is owned by uid <n>`).

**Verified by**: `profiles::a_profile_others_can_write_is_refused`.

#### Scenario: Writable by others
- **WHEN** `profiles/demo.toml` has mode 0666 and the owner calls `pane.create {profile: "demo"}`
- **THEN** the reply is `bad_request` with a message starting `create pane: profile demo: ` that names the file and says it is writable by others, and no pane is listed

### Requirement: A caller names a profile, never a path
`pane.create.profile` and every profile name SHALL match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`; an empty `profile` SHALL mean no profile. Any other value MUST be refused with `bad_request` `create pane: bad profile name: <name>` before the server touches the file system; a valid name without a file SHALL be refused with `bad_request` `create pane: no such profile: <name>`; no pane SHALL start and the connection SHALL stay open.

**Verified by**: `profiles::a_profile_name_is_never_a_path`, `profiles::an_unknown_profile_is_an_error`.

#### Scenario: Traversal
- **WHEN** the owner calls `pane.create` with `profile` `../server`, `/etc/passwd`, `.hidden` or `a/b`
- **THEN** each is answered `bad_request` `create pane: bad profile name: <that value>` and no pane is listed

#### Scenario: Unknown name
- **WHEN** no `nope.toml` exists and the owner calls `pane.create {profile: "nope"}`
- **THEN** the reply is `bad_request` `create pane: no such profile: nope` and a pane without a profile can be created next

### Requirement: Profiles are parsed strictly
A profile file SHALL be TOML with only the keys `description` (optional), `command` (strings), `cwd`, `[env]` (string values) and `[sandbox]` with `read_only`, `read_write`, `executables`, `env` (strings) and `network` (`none` default, `client`, `server`). An unknown key at any level or a value of the wrong type SHALL make the profile invalid, and `pane.create` with it MUST be refused with `bad_request` starting `create pane: profile <name>: ` naming the file and the reason.

**Verified by**: `profiles::a_misspelt_key_makes_the_profile_invalid`, unit test `profile::tests::every_sandbox_key_is_parsed` (crates/tilrd/src/profile.rs).

#### Scenario: Misspelt key
- **WHEN** `demo.toml` contains `comand = ["/bin/sh"]` and the owner creates a pane with it
- **THEN** the reply starts with `create pane: profile demo: ` and names the file and `comand`, and no pane is listed

#### Scenario: No description
- **WHEN** a profile has only `command = ["/bin/sh"]`
- **THEN** it is valid and `pane.profiles` lists it with an empty `description`

### Requirement: Paths and variable names in a profile
`cwd` and every path in `[sandbox]` MUST be absolute or start with `~` (the user's home from the password database); `.` and `..` components SHALL be folded lexically. A relative path, a path with NUL or newline, or an `[env]` name that is empty, contains `=` or starts with `TILR_`, SHALL make the profile invalid.

**Verified by**: unit tests `profile::tests::relative_paths_are_refused`, `profile::tests::tilde_is_the_home_directory`, `profile::tests::tilr_variables_cannot_be_set` (crates/tilrd/src/profile.rs).

#### Scenario: Relative path
- **WHEN** a profile says `cwd = "work"`
- **THEN** creating a pane with it is refused, naming `cwd` and saying the path must be absolute or start with `~`

#### Scenario: A server variable
- **WHEN** a profile says `[env] TILR_PANE = "x"`
- **THEN** the profile is invalid and the reason names `TILR_PANE`

### Requirement: A pane names its profile
`PaneInfo.profile` of a pane created with a profile SHALL be the profile's name, in the reply of `pane.create`, in `pane.list` and in the `pane.*` events, for as long as the pane runs; it SHALL be empty for a pane created without a profile and for a pane of `tilr.v1`. `PaneInfo.sandboxed` SHALL be true only for a pane created with a profile that has a `[sandbox]` table.

**Verified by**: `profiles::a_pane_starts_with_the_profile_from_the_config_directory`.

#### Scenario: Listed with its profile
- **WHEN** the owner calls `pane.create {profile: "demo"}` with a valid unsandboxed `demo.toml`, then `pane.create` without a profile
- **THEN** the first reply and `pane.list` name `demo` with `sandboxed` false, and the second reply's `profile` is empty

### Requirement: A profile is read on every create
The server SHALL read and validate the profile file on every `pane.create` that names it and SHALL NOT cache it; a change to the file SHALL apply to the next pane created with it and SHALL NOT change a pane that already runs.

**Verified by**: `profiles::an_edited_profile_applies_to_the_next_pane_only`.

#### Scenario: Edit between two panes
- **WHEN** a pane is created with profile `demo` (`[env] X = "1"`), the file is changed to `X = "2"`, and a second pane is created with it
- **THEN** `echo $X` prints `1` in the first pane and `2` in the second

### Requirement: The request's command wins and the profile's sandbox stays
A pane created with a profile SHALL run the request's `command` if it is not empty, else the profile's `command`, else the login shell; it SHALL start in the request's `cwd` if that is an existing directory, else the profile's `cwd` if that is an existing directory, else the home directory. No request field SHALL change `[env]` or `[sandbox]`: a pane of a sandboxed profile SHALL be sandboxed whatever command the request gives.

**Verified by**: `profiles::request_fields_override_the_profile`, `sandbox::a_command_from_the_request_runs_in_the_profiles_sandbox`.

#### Scenario: Request wins
- **WHEN** a profile has `command = ["/bin/sh"]` and `cwd = "<dir>"` and is used with `command` `["/bin/bash", "--norc"]` and another existing `cwd`
- **THEN** `/bin/bash --norc` runs in that directory and `PaneInfo.profile` names the profile

#### Scenario: Sandbox stays
- **WHEN** a sandboxed profile with `read_write = ["<w>"]` is used with `command` `["/bin/sh", "-c", "touch <w>/a <o>/b"]`
- **THEN** `<w>/a` exists, `<o>/b` does not, and `PaneInfo.sandboxed` is true

### Requirement: Variables of a profile pane
A pane created with an unsandboxed profile SHALL get the server's environment with the profile's `[env]` set over it (a value starting with `~/` expanded to the home directory), and then the terminal variables `TERM`, `COLORTERM`, `SHELL` and `TILR_PANE` as for any pane.

**Verified by**: `profiles::request_fields_override_the_profile`.

#### Scenario: TERM stays the server's
- **WHEN** a profile sets `[env] TERM = "dumb"` and `GREETING = "hi"`
- **THEN** in the pane `echo $GREETING` prints `hi` and `TERM` is `xterm-256color`

### Requirement: A sandboxed profile is refused where it cannot be enforced
On a server whose `server.info.features` does not list `sandbox`, `pane.create` with a profile that has a `[sandbox]` table MUST be refused with `bad_request` `create pane: profile <name> asks for a sandbox, which this server cannot enforce`, and the pane's program MUST NOT be started, restricted or not.

**Verified by**: `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`.

#### Scenario: No sandbox support
- **WHEN** a server without the `sandbox` feature (failpoint `sandbox-unavailable`) gets `pane.create` with a profile that has `[sandbox] read_write = ["~/w"]` and `command = ["/bin/sh", "-c", "touch <marker>"]`
- **THEN** the caller receives that error, no pane is listed, and `<marker>` does not exist

### Requirement: Listing profiles
`pane.profiles {}` SHALL answer, sorted by name, every `*.toml` of the profiles directory whose stem is a valid profile name, with `name`, `description`, `sandboxed` (the file has a `[sandbox]` table) and `error` (empty when the profile is valid, else the reason `pane.create` would give), at most 1000, each text cut at 1 KiB; other files SHALL be left out; without a directory the list SHALL be empty.

**Verified by**: `profiles::profiles_are_listed_with_their_errors`.

#### Scenario: Three files
- **WHEN** the directory holds `a.toml` (valid), `b.toml` (with `[sandbox]`), `c.toml` (bad TOML) and `.x.toml`
- **THEN** `pane.profiles` lists `a`, `b`, `c` in that order, `b` with `sandboxed` true, `c` with a non-empty `error`, and no `.x`
