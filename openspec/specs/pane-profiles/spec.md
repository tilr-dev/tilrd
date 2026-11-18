# pane-profiles Specification

## Purpose
A launch profile is a named TOML file in `profiles_dir` that says how a pane starts: its
command, cwd, environment and sandbox. `pane.create {profile}` starts a pane from one, read
afresh on every create, with the request's command and cwd taking precedence over the
profile's; `pane.profiles` lists the profiles and the error of each invalid one. Because a
profile decides what runs in a pane, tilrd trusts a profile file only as sshd trusts
`authorized_keys`: the file and every directory above it up to the home directory belong to
the user or root and are not writable by group or others, and a caller names a profile,
never a path. A profile with a sandbox is refused until the sandbox can be enforced.

Verified by `crates/tilrd/tests/profiles.rs` and the unit tests of `crates/tilrd/src/profile.rs`.

## Requirements

### Requirement: Profiles live in profiles_dir
A profile named `<name>` SHALL be the file `<name>.toml` in the directory `profiles_dir` of `server.toml` (absolute or starting with `~`), by default `profiles/` in the directory of the server's configuration file (`$TILR_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tilr`, else `~/.config/tilr`; with `--config PATH` or `TILR_CONFIG`, the directory of `PATH`). A missing directory SHALL mean the server has no profiles.

**Verified by**: `profiles::a_pane_starts_with_the_profile_from_the_config_directory`, `profiles::profiles_dir_names_another_directory`.

#### Scenario: Default directory
- **WHEN** the server runs with `TILR_CONFIG_DIR=<dir>`, `<dir>/profiles/demo.toml` says `command = ["/bin/sh"]`, and the owner calls `pane.create {profile: "demo"}`
- **THEN** the reply's `PaneInfo.profile` is `demo` and its `command` is `["/bin/sh"]`

#### Scenario: Own directory
- **WHEN** `server.toml` says `profiles_dir = "<other>"` and `<other>/demo.toml` exists, and `<dir>/profiles/demo.toml` does not
- **THEN** `pane.create {profile: "demo"}` starts the pane from `<other>/demo.toml`

### Requirement: The profiles directory is absolute
The server SHALL resolve the profiles directory once at start to an absolute path: a default next to a relative configuration file is made absolute against the directory the server starts in, and a `profiles_dir` that is neither absolute nor starts with `~` SHALL stop the server from starting.

**Verified by**: unit tests `config::tests::profiles_dir_defaults_next_to_the_configuration_file`, `config::tests::a_relative_configuration_file_gives_an_absolute_profiles_dir` (crates/tilrd/src/config.rs).

#### Scenario: Relative configuration file
- **WHEN** the server starts with `--config conf/my.toml`
- **THEN** its profiles directory is `<the directory it started in>/conf/profiles`

#### Scenario: Relative profiles_dir
- **WHEN** `server.toml` says `profiles_dir = "relative"`
- **THEN** the server does not start and the error names `profiles_dir`

### Requirement: Profile files are the user's alone
As OpenSSH's StrictModes checks `authorized_keys`, the server SHALL open a profile file once without blocking and judge the opened file (`fstat`): it MUST be a regular file (a symlink to one is allowed), owned by the server's user or root, and not writable by group or others. It SHALL then read at most 64 KiB from that descriptor. Otherwise the profile SHALL NOT load and `pane.create` and `pane.profiles` SHALL name the file and the reason.

**Verified by**: `profiles::a_profile_others_can_write_is_refused`, `profiles::a_profile_larger_than_the_bound_is_refused`, `profiles::a_fifo_in_place_of_a_profile_is_refused_without_hanging`, unit test `profile::tests::only_the_user_or_root_may_own_a_profile_or_a_directory_above_it` (crates/tilrd/src/profile.rs).

#### Scenario: Writable by others
- **WHEN** `profiles/demo.toml` has mode 0666 and the owner calls `pane.create {profile: "demo"}`
- **THEN** the reply is `bad_request` with a message starting `create pane: profile demo: ` that names the file and says it is writable by others, and no pane is listed

#### Scenario: Owned by root or another user
- **WHEN** a profile file or a directory above it is owned by root, or by a uid other than the server's
- **THEN** root's is accepted, and the other one's is refused with `<path> is owned by uid <n>`

#### Scenario: Too large
- **WHEN** a profile file has more than 65536 bytes
- **THEN** it is refused with `<file> is larger than 65536 bytes`

#### Scenario: A FIFO
- **WHEN** `profiles/pipe.toml` is a FIFO nobody writes into
- **THEN** `pane.create {profile: "pipe"}` answers at once with `<file> is not a regular file`

### Requirement: The directories above a profile are the user's alone
Every directory of a profile file's real path (after symlinks) and of the real profiles directory, from the directory itself up to `/`, or up to and including the user's home directory when the path is under it, MUST be owned by the server's user or root and not writable by group or others, a sticky directory such as `/tmp` included, as sshd's `auth_secure_path` requires; otherwise the profile SHALL NOT load and the error SHALL name the directory.

**Verified by**: `profiles::a_symlinked_profile_in_a_group_writable_directory_is_refused`, `profiles::a_group_writable_directory_above_the_profiles_is_refused`, unit test `profile::tests::the_directories_above_are_checked_up_to_the_home_directory` (crates/tilrd/src/profile.rs).

#### Scenario: A symlink into a group-writable directory
- **WHEN** `profiles/demo.toml` is a symlink to a file of mode 0600 in a directory of mode 0770
- **THEN** `pane.create {profile: "demo"}` is refused with `create pane: profile demo: <that directory> is writable by others` and `pane.profiles` gives the same error

#### Scenario: A group-writable directory above the profiles directory
- **WHEN** `profiles_dir` is `<parent>/profiles` and `<parent>` has mode 0770
- **THEN** `pane.create` with a profile there is refused naming `<parent>`

#### Scenario: Above the home directory
- **WHEN** the profiles directory is under the home directory and a directory above the home directory is writable by group
- **THEN** the profile loads; a group-writable home directory itself is refused

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
- **THEN** the first reply, `pane.list` and the `pane.created` event name `demo` with `sandboxed` false, and the second reply's `profile` is empty

### Requirement: A profile is read on every create
The server SHALL read and validate the profile file on every `pane.create` that names it and SHALL NOT cache it; a change to the file SHALL apply to the next pane created with it and SHALL NOT change a pane that already runs.

**Verified by**: `profiles::an_edited_profile_applies_to_the_next_pane_only`.

#### Scenario: Edit between two panes
- **WHEN** a pane is created with profile `demo` (`[env] X = "1"`), the file is changed to `X = "2"`, and a second pane is created with it
- **THEN** `echo $X` prints `1` in the first pane and `2` in the second

### Requirement: The request's command and cwd win
A pane created with a profile SHALL run the request's `command` if it is not empty, else the profile's `command`, else the login shell; it SHALL start in the request's `cwd` if that is an existing directory, else the profile's `cwd` if that is an existing directory, else the home directory. Nothing in the request SHALL change the profile's `[env]` or `[sandbox]`.

**Verified by**: `profiles::request_fields_override_the_profile`.

#### Scenario: Request wins
- **WHEN** a profile has `command = ["/bin/sh"]` and `cwd = "<dir>"` and is used with `command` `["/bin/bash", "--norc"]` and another existing `cwd`
- **THEN** `/bin/bash --norc` runs in that directory and `PaneInfo.profile` names the profile

#### Scenario: The profile's environment stays
- **WHEN** a profile with `[env] GREETING = "hi"` is used with a `command` of its own
- **THEN** that command runs with `GREETING` set to `hi`

### Requirement: Variables of a profile pane
A pane created with an unsandboxed profile SHALL get the server's environment with the profile's `[env]` set over it (a value starting with `~/` expanded to the home directory), and then the terminal variables `TERM`, `COLORTERM`, `SHELL` and `TILR_PANE` as for any pane.

**Verified by**: `profiles::request_fields_override_the_profile`.

#### Scenario: TERM stays the server's
- **WHEN** a profile sets `[env] TERM = "dumb"` and `GREETING = "hi"`
- **THEN** in the pane `echo $GREETING` prints `hi` and `TERM` is `xterm-256color`

### Requirement: A sandboxed profile is refused where it cannot be enforced
On a server whose `server.info.features` does not list `sandbox`, `pane.create` with a profile that has a `[sandbox]` table MUST be refused with `bad_request` `create pane: profile <name> asks for a sandbox, which this server cannot enforce`, and the pane's program MUST NOT be started, restricted or not. The path that starts the pane SHALL itself refuse a pane that is to run in a sandbox it does not apply, so that a server which claims to enforce a sandbox cannot start such a pane unconfined.

**Verified by**: `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`, unit tests `supervisor::bus::core::profiles::tests::prepare_create_refuses_a_sandbox_it_cannot_enforce`, `supervisor::bus::core::panes::tests::the_start_path_refuses_a_sandbox_it_does_not_apply`.

#### Scenario: No sandbox support
- **WHEN** a server without the `sandbox` feature gets `pane.create` with a profile that has `[sandbox] read_write = ["~/w"]` and `command = ["/bin/sh", "-c", "touch <marker>"]`
- **THEN** the caller receives that error, no pane is listed, and `<marker>` does not exist

### Requirement: Listing profiles
`pane.profiles {}` SHALL answer, sorted by name, every `*.toml` of the profiles directory whose stem is a valid profile name, with `name`, `description`, `sandboxed` (the file read has a `[sandbox]` table, even if invalid otherwise) and `error` (empty when valid, else the reason `pane.create` would give), at most 1000, each text cut at 1 KiB; other files SHALL be left out; without a directory the list SHALL be empty.

**Verified by**: `profiles::profiles_are_listed_with_their_errors`, unit test `profile::tests::the_listing_is_sorted_with_errors_and_skips_other_files` (crates/tilrd/src/profile.rs).

#### Scenario: Three files
- **WHEN** the directory holds `a.toml` (valid), `b.toml` (with `[sandbox]`), `c.toml` (bad TOML) and `.x.toml`
- **THEN** `pane.profiles` lists `a`, `b`, `c` in that order, `b` with `sandboxed` true, `c` with a non-empty `error`, and no `.x`

#### Scenario: Invalid with a sandbox
- **WHEN** `d.toml` has a misspelt key and a `[sandbox]` table
- **THEN** `pane.profiles` lists `d` with an `error` naming the key and `sandboxed` true
