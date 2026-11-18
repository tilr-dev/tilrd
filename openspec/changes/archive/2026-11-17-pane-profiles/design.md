# Design: Launch profiles for panes

Shared design: `openspec/design/unified-bus.md` Part P (P1-P8) and §S1 (the sandbox is asked
for only by naming a profile). This change's part follows.

## Decisions

- **Snapshot type now, carried later.** `profile.rs` defines the resolved snapshot (argv, cwd,
  env, sandbox) that the pane keeps; `bus-reload` writes it into `PaneState` (supervisor-state 3).
- **Sandboxed profiles refused.** Without the `sandbox` feature every profile with `[sandbox]`
  is refused with P7's text; `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`
  needs no failpoint here; the sandbox changes decide how they test it on a server that
  can enforce one.
- **Fail closed twice.** `profiles::sandbox_enforceable()` is false, and besides it the
  start path (`panes::start_pane`) takes the snapshot's `Option<&SandboxSpec>` and refuses
  `Some` (`create pane: this server cannot apply a sandbox`) until a sandbox change applies
  it there: flipping the constant alone cannot start an unconfined pane that reports
  `sandboxed: true`.
- **StrictModes as sshd** (owner decision: "do it the way ssh does with keys").
  `profile::read_text` follows `auth_openfile` and `auth_secure_path`: open once with
  `O_RDONLY|O_CLOEXEC|O_NOCTTY|O_NONBLOCK` (a FIFO cannot hold the call), `fstat` the
  descriptor (regular file, owner the server's euid or root, no group/other write), check
  that the canonical path still names that inode, walk the directories of the canonical
  file path and of the canonical profiles directory up to `/`, stopping after the home
  directory from the password database as sshd stops at `pw_dir` (the home itself is
  checked); read at most 64 KiB + 1 from the same descriptor and refuse a larger file.
  Sticky world-writable directories (`/tmp`) are refused, as in sshd. `pane.profiles`
  uses the same loader. The pure check `profile::secure(path, owner, mode, euid)` is unit
  tested for owners a test cannot make (root, another uid).
- **Test trees.** The integration tests keep their profiles and the configuration
  directory (`TILR_CONFIG_DIR`) under `CARGO_TARGET_TMPDIR` (`bus_profiles::secure_dir`),
  which passes the walk on macOS (under the home) and in Linux CI and the container
  (owned by the user or root up to `/`); the system's `/tmp` would not. Unit tests build
  their tree in the temporary directory and pass its root as the home directory, where
  the walk stops (`profile::tests::Scratch`).
- **`sandboxed` in a listing of an invalid profile**: when the file was read but does
  not load, a lenient `toml::Table` parse of the same text says whether it has a
  `[sandbox]` table; text that is not TOML, or a file not read, has none.
- **Absolute profiles directory.** `config::profiles_dir` returns an absolute path: the
  key must be absolute or `~` (else the server does not start), the default is made
  absolute against the starting directory; with no configuration file path at all the
  server has no profiles.
- **`pane.profiles` in the method list.** `bus.methods` lists `pane.profiles` with the
  provider `tilrd` (checked in `profiles::a_pane_starts_with_the_profile_from_the_config_directory`;
  `bus_terminal` has no test of the method list to extend).

- **Where the directory comes from** *(added by the split)*. `main.rs` resolves `profiles_dir`
  once (`config::profiles_dir`) and stores it with `profile::set_dir`; `profile::dir()` reads
  it. A reload execs a new process, which resolves it again from its own configuration. No
  field was added to `supervisor::Options`, so `supervisor.rs` stays untouched.
- **The snapshot type** *(added by the split)*: `profile::Snapshot { name, argv, cwd, env,
  sandbox: Option<SandboxSpec> }` with `SandboxSpec { read_only, read_write, executables, env,
  network }` and `Network { None, Client, Server }`, all `Serialize + Deserialize` for
  `bus-reload`; paths are resolved (absolute, `~` expanded, `.`/`..` folded).
- **The seam for `pane.create`** *(added by the split)*: `bus/core/profiles.rs`
  `prepare_create(authz, ctx, &CreateRequest) -> Result<Option<Resolved>, BusError>` asks the
  authorizer for `PaneCreate`, then resolves the profile (names, file, refusal of a sandbox,
  override rules). `panes.rs`'s `pane.create` handler calls it in the handler's synchronous
  part, before the supervisor's part is queued (the call keeps its place in its connection's
  order, B5), and starts the pane from `Resolved { snapshot, command, cwd, env }`; the file
  is read there, as `start_dir` stats a directory on the supervisor's task for `tilr.v1`.
- **The pane's snapshot** *(added by the split)*: `bus/core/profiles.rs` keeps it in a slot
  (`ProfilesState`, by pane) from the start of the pane until it exits or fails to start;
  `PaneInfo.profile` and `PaneInfo.sandboxed` come from it. Carrying it across a reload is
  `bus-reload`'s: until then a pane keeps running what its profile said, and its
  `PaneInfo.profile` is empty after a reload.

## Reload and reconnect

Profiles are read on create only; nothing is cached. The snapshot crosses a reload with
`bus-reload`.

## Backpressure

None new: at most 1000 profiles listed, texts cut at 1 KiB (P6).
