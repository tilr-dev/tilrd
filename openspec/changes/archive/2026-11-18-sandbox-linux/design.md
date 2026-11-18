# Design: Sandboxed panes on Linux

Shared design: `openspec/design/unified-bus.md` Part S: §S1, §S2, §S5 (Landlock plus seccomp),
§S6, §S7, §S8, §S9. This change's part follows.

## Decisions

- **Neutral module first.** `sandbox/mod.rs` holds what both platforms share (S6-S9) behind
  `trait Platform { fn check() -> Result<Features, String>; fn prepare(&[Grant], Network,
  Features, &Dirs) -> Result<Prepared, String>; fn apply_in_child(&Prepared) -> io::Result<()>; }`
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
- **The way into the pane process.** `pane.create` hands the request over, with the server's
  `sandbox::Dirs` (the pane process cannot know a `--config` elsewhere), as a
  `sandbox::Handover`: a file `<runtime dir>/sandbox/<pane id>.request` (0600, in the
  0700 `sandbox` directory of the protected runtime directory, beside the pane's `TMPDIR`, not
  in it), and starts the pane process with the arguments it returns, `--sandbox <file>`, beside
  `--id`. The request itself is not on the command line: any process of the user reads that
  (`ps`, `/proc/<pid>/cmdline`), and `[env]` may hold a token. A reload execs the pane process
  with the same arguments; the file stays until the pane ends and goes with its `TMPDIR`
  (and at server start, with the leftovers of panes that do not run). Rejected: an inherited
  descriptor (it would have to survive the pane process's own exec at a reload, and nothing
  else crosses that way). The pane process builds everything before the
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
- **The filter, where S5 is silent, fails closed.** Besides `socket(AF_UNIX)`: `socketpair`
  only for `AF_UNIX` streams and sequenced packets (an unbound datagram socket from a pair
  can `sendto` any named Unix socket, and an abstract one where Landlock does not scope them,
  ABI < 6); netlink only `NETLINK_ROUTE` (what `getaddrinfo` reads; `sock_diag` would list
  every socket of the host); io_uring refused (its socket and connect operations bypass the
  filter); `listen` refused with `none` and `client` (it binds an unbound TCP socket to a free
  port, which Landlock's `BIND_TCP` does not see); with `none` every socket but netlink is
  refused, with `client` every IPv4 or IPv6 socket but TCP streams and datagrams (SCTP and
  MPTCP streams are not under Landlock's TCP rules); a syscall of another ABI (32-bit) kills
  the process, an x32 one gets `ENOSYS`. The child marks every descriptor above 2
  close-on-exec first, so no socket the pane process holds reaches the program.
- **No namespaces, and what Docker refuses.** `unshare` and `clone` with a namespace flag get
  `EPERM`, `clone3` `ENOSYS` (its flags are behind a pointer a filter cannot read; glibc falls
  back to `clone`), as Docker's default profile and Flatpak do: a user namespace gives the pane
  every capability inside it, a large kernel surface and a way to private mounts. `keyctl`,
  `add_key`, `request_key` (the user's keyrings hold credentials of other sessions), `bpf`,
  `perf_event_open`, `userfaultfd` and `setns` get `EPERM`, as Docker does. The cost: rootless
  podman, bubblewrap, Chrome's and Electron's own sandboxes and `unshare -r` fail inside a
  sandboxed pane (Chrome runs with `--no-sandbox`); Kerberos' `KEYRING:` caches do not work.
  Rejected: allowing user namespaces (a bubblewrap-based sandbox makes its namespaces before
  the program runs, and refuses new ones inside with `--disable-userns`).
- **Two tables, one filter.** The syscall numbers the filter uses are written for x86_64 and
  aarch64 both, so the tests run the filter of each in a small classic-BPF interpreter
  whatever the host (every jump inside the program, every path ends in a return, the answer
  for each syscall the sandbox refuses), and check the host's table against `libc`.
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
- **Inner terminals: the default stays (owner, 2026-11-17).** No `ptys` flag and no broker. A
  profile that needs tmux or `script` inside grants `/dev/pts` in `read_write` (`/dev/ptmx`
  is in the base devices already), or the pane runs without a profile. The price, said in the
  README: such a pane can open every terminal of the same user, read what is typed there and
  write to it; the sandbox no longer isolates terminals. The server logs a warning for each
  pane whose grant (`read_only` or `read_write`) covers `/dev/pts` or lies in it.
- **`/etc/resolv.conf`.** With `client` or `server`, the file it links to is readable when
  that is outside `/etc` (systemd-resolved's `/run/...`): name lookups work.
- **Grants inside a protected directory** are refused like grants over one (a grant of the
  state directory's token is no better than a grant of the directory). Paths are compared as
  written (folded) and with symlinks resolved, when the request is planned; then each grant
  is opened once (`O_PATH`) and checked again by what was opened (the kernel's path of the
  descriptor, `/proc/self/fd/N`, and its device and inode against each protected directory),
  and that descriptor is the Landlock rule, so a path swapped for a link between the check and
  the rule cannot slip a server directory in.
- **The server's executable** (`current_exe()`, links resolved) is protected from `read_write`
  grants that cover it: a reload execs it outside any sandbox. `read_only` may cover it.
- **The directories of a server started elsewhere.** `Dirs::current()` knows the default
  configuration directory; `Dirs::protect_config_file` (the `--config` file's directory) and
  `Dirs::protect_profiles_dir` are the API the wiring of `pane.create` calls (tasks 2.2).
- **Limits, documented rather than closed.** Landlock has no right for metadata: a pane can
  `chmod`, `chown` to its groups, `touch` and `setxattr` any file of the user, the token and
  the state directory included (it cannot read them). An inotify watch on a directory that is
  not granted tells the names made there. What the pane can read it can run (a copy in its
  `TMPDIR`, `memfd_create` and `execve`, `ld.so`): `executables` and the program are grants to
  read and run, not an allow list of programs. A `read_write` grant can leave what runs later
  outside the sandbox (`.git/hooks`, `.envrc`, shell rc files, editor configuration): the
  sandbox restricts the pane, not what the user later runs from the files it wrote. Container
  sandboxes (Docker, Flatpak, bubblewrap) hide what they do not mount, so they have no
  metadata or inotify limit outside the mounts; Landlock cannot hide, it can only refuse.
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
