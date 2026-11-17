# Design: Sandboxed panes on Linux

Shared design: `openspec/design/unified-bus.md` Part S: §S1, §S2, §S5 (Landlock plus seccomp),
§S6, §S7, §S8, §S9. This change's part follows.

## Decisions

- **Neutral module first.** `sandbox/mod.rs` holds what both platforms share (S6-S9) behind
  `trait Platform { fn check() -> Result<Features, String>; fn prepare(&Snapshot, &PaneDirs) ->
  Result<Prepared, String>; fn apply_in_child(&Prepared) -> Result<(), Errno>; }`
  *(added by the split)*; `linux.rs` implements it; `sandbox-macos` adds `macos.rs`.
- **Failpoint for the profile test.** `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`
  (`pane-profiles`) switches to the failpoint `sandbox-unavailable` here.
- **macOS later.** Requirements that name macOS (feature check by `sandbox-exec`, Unix sockets
  on macOS, `sandbox-exec-missing`) are verified on macOS by `sandbox-macos`, which runs every
  `sandbox::*` test there.

- **Input of its own.** The sandbox takes a `sandbox::Request` (plain data: `read_only`,
  `read_write`, `executables`, `pass_env`, `network`, and `env`, the profile's `[env]`, which
  S6 sets last), not the profile's `Snapshot`: `pane-profiles` maps its `SandboxSpec` and
  `[env]` onto it at the call site. `executables` takes a name (looked up in the `PATH` of
  `env`, else the server's, skipping directories under the protected ones) or an absolute
  path; a name not found is skipped with a log line, as a missing grant path is (S7).
- **The way into the pane process.** `pane.create` starts the pane process with
  `tilrd pane --sandbox <the request as JSON>` beside `--id`; a reload execs it with the same
  arguments, so nothing new crosses a reload. The pane process builds everything before the
  fork (`sandbox::plan`: the grant checks, the `TMPDIR`, the environment, a Landlock ruleset
  descriptor and a BPF program); the child only installs them with raw syscalls.
- **Hooks outside the owned files.** `pane.rs`: the `--sandbox` argument, passing it to
  `pty::spawn_shell`, removing `<runtime dir>/sandbox/<pane id>/` when the pane ends.
  `supervisor.rs`: `sandbox::at_server_start` after the panes of a reload are resumed (the
  leftovers of panes that do not run, and a log line saying whether the sandbox is offered).
  `main.rs`: the module.
- **Crates.** `landlock` (the Landlock maintainers' crate, `HardRequirement`: every right of
  the ABI handled, none downgraded) builds the ruleset. The seccomp filter is about 30
  instructions assembled on `libc`'s definitions, with no crate: a generic one would not add
  the x32 check, and nothing else is needed.
- **The filter, where S5 is silent, fails closed.** Besides `socket(AF_UNIX)`: io_uring is
  refused (its socket and connect operations bypass the filter); `listen` is refused with
  `none` and `client` (it binds an unbound TCP socket to a free port, which Landlock's
  `BIND_TCP` does not see); with `none` every socket but netlink is refused, with `client`
  every IPv4 or IPv6 socket but TCP streams and datagrams (SCTP and MPTCP streams are not
  under Landlock's TCP rules); a syscall of another ABI (32-bit) kills the process, an x32
  one gets `ENOSYS`. The child marks every descriptor above 2 close-on-exec first, so no
  socket the pane process holds reaches the program.
- **Only the pane's own terminal.** S5's base grant of `/dev/pts` is narrowed: a process of
  the same user could open any `/dev/pts/M` of the user (another pane's) and read what is
  typed there. The pane process grants the pane's slave by name (`ttyname` of the slave it
  opened, a file rule, before the restriction) beside `/dev/tty` and `/dev/ptmx`. The cost: a
  terminal made inside the pane cannot be used, since Landlock checks the open of its slave
  by path and through `TIOCGPTPEER` alike, so `openpty`, `script` and tmux fail there (S5's
  line "read+write on … `/dev/pts`" in the shared design is to be corrected to this).
  Rejected: the blanket grant (reads other terminals); a private devpts mount (needs a user
  namespace, rejected in S5); a broker in the pane process that opens slaves for the pane
  (a new channel into the sandbox, not designed).
- **`/etc/resolv.conf`.** With `client` or `server`, the file it links to is readable when
  that is outside `/etc` (systemd-resolved's `/run/...`): name lookups work.
- **Grants inside a protected directory** are refused like grants over one (a grant of the
  state directory's token is no better than a grant of the directory). Paths are compared as
  written (folded) and with symlinks resolved.
- **Tests below the bus first.** Until `pane.create` takes profiles, `src/sandbox/tests.rs`
  starts real programs under the sandbox on a pty with `pty::spawn_with`, the function the
  pane process uses, and the bus-level tests of the spec come at the merge (tasks 2.x). The
  Linux test container gets `vim-tiny` and `less`, which the GitHub runner has.

## Reload and reconnect

The Landlock domain and seccomp filter stay on the shell and its descendants across the
pane process's exec (they are attached in the shell child, not in the pane process). `PaneInfo.
sandboxed` and the snapshot cross with `bus-reload`.

## Backpressure

None new.
