# Design: Sandboxed panes

## Context

The request is the pane's profile `[sandbox]` (change `pane-profiles`): `read_only`,
`read_write`, `network` (`none` | `client` | `server`), `executables`, `env`, with absolute
paths after `~` expansion, and the snapshot kept in `PaneState` across reloads. At 14388b16 the
supervisor runs `tilrd pane …` with `setsid` only; the pane process opens the pty and runs the
program from `pty.rs` `spawn_shell`, whose `pre_exec` does `setsid` and `TIOCSCTTY`; the login
shell goes through the wrappers and shims (`shell.rs`, `TILR_SHIMS`), and the pane's environment
carries the host-events socket, the server socket and (with agent forwarding) `SSH_AUTH_SOCK`.

The prototype (`runtime-tmp`, `docs/how-it-works.md` §12) and the probe (D10) show what an App Sandbox helper costs
on the pty: `tcsetattr`, `tcgetpgrp`/`tcsetpgrp`, `TIOCGETD` refused, hence no raw mode and no job
control; quarantine on files created inside; a container per spec hash, never cleaned; `ps`
refused; `~/.zshrc` unreadable without a grant. Those are the failure modes this design must not
have.

## Goals / Non-goals

Goals: a sandboxed pane that is a normal pane to its user; enforcement of the profile's grants on
Linux and macOS; no way from inside to the server or to the user's other privileges; fail
closed; survive reload; clean up.

Non-goals: see proposal. Panes moving to a separate `tilr-panes` component later (2026-10-26
direction) does not change this design: the mechanism sits where the program is spawned.

## Decisions

History: D1-D3 below are the 2026-10-18 draft, kept; their status is marked.

### D1. Quarantine experiment (2026-10-18) — answered
The prototype ran it: files created inside an App Sandbox get `com.apple.quarantine` and cannot be
executed inside; removing the attribute from outside makes them run. It is now requirement
"Programs made inside run inside" of `pane-sandbox`, which the macOS mechanism must meet.

### D2. macOS (2026-10-18) — superseded by D10
"The supervisor (not sandboxed) spawns the sandboxed pane binary instead of `tilrd pane`" stays
not taken: under Seatbelt the pane process stays unrestricted and only the program is sandboxed (D10).

### D3. Linux (2026-10-18) — kept, refined by D5
In the pane's `pre_exec` before the shell: build a Landlock ruleset from the request, `landlock_restrict_self`. Children inherit it. If the kernel's ABI is too old for the request, `CreatePane` fails with a message; `sandbox` is not listed when Landlock is absent.

### D4. The request comes from the profile (2026-11-17)
No sandbox fields in `CreatePane`; a client asks for a sandbox only by naming a profile. One
place to read and review what an agent may do; the bus (`bus-panes`) uses the same names.

### D5. Linux mechanism: Landlock plus a seccomp filter
Applied in the shell child's `pre_exec` after `setsid`/`TIOCSCTTY`, before `execve`:

- `prctl(PR_SET_NO_NEW_PRIVS)`, then a Landlock ruleset handling every filesystem right of the
  kernel's ABI (no best-effort downgrade: the handled set is what the ABI offers, and the
  `sandbox` feature needs ABI ≥ 4, kernel 6.7):
  - `read_only` paths: read file, read dir, execute.
  - `read_write` paths: all filesystem rights (so scripts and binaries made there run; no
    quarantine on Linux).
  - `executables` (resolved through the pane's `PATH` at creation) and the program actually run
    (the command's `argv[0]` or the login shell): read and execute on the file; their ELF
    interpreter and libraries come from the always-readable system directories.
  - Always granted (the "base"): read and execute on `/usr`, `/bin`, `/sbin`, `/lib`, `/lib32`,
    `/lib64`, `/etc` (read only), `/proc`, `/sys` (read only); read and write on `/dev/null`,
    `/dev/zero`, `/dev/full`, `/dev/random`, `/dev/urandom`, `/dev/tty`, `/dev/ptmx`, `/dev/pts`
    (with `IOCTL_DEV` where ABI ≥ 5, so `stty` and raw mode keep working on a reopened tty); the
    pane's private temporary directory (D8).
  - Network (ABI ≥ 4): `client` handles `BIND_TCP` with no port allowed (no listening),
    `CONNECT_TCP` free; `none` handles both with nothing allowed; `server` handles neither.
  - ABI ≥ 6: `LANDLOCK_SCOPE_SIGNAL` and `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET` (no signals to,
    no abstract sockets of, processes outside the sandbox).
- A seccomp-bpf filter (installed after Landlock): `socket(AF_UNIX, …)` → `EACCES`
  (`socketpair` stays allowed) — Landlock does not govern `connect` to a named Unix socket, and
  the server's, host-events and agent sockets accept any process of the user (`access-control`:
  peer uid check). With `network = "none"` also `socket(AF_INET | AF_INET6, SOCK_DGRAM | SOCK_RAW,
  …)` → `EACCES` (Landlock has no UDP rules).
- Job control and raw mode are untouched: Landlock does not filter ioctls on descriptors opened
  before the restriction (fds 0-2 are the pty slave), and `TIOCSPGRP`/`tcsetattr` are not
  syscalls seccomp blocks here.

Rejected: bubblewrap / user namespaces (unprivileged user namespaces are disabled or
AppArmor-restricted on Ubuntu ≥ 23.10; a setuid helper is out of scope); seccomp-only (no path
rules).

### D6. What a sandboxed pane does not get
- Environment built from scratch: `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `COLORTERM`,
  `TILR_PANE`, `TMPDIR` (D8), `PATH`, the variables named in `[sandbox] env` taken from the
  server's environment, then `[env]`. `PATH` default: the directories of `executables`, then
  `/usr/bin:/bin:/usr/sbin:/sbin` (the prototype's default). Never set: `TILR_SHIMS`,
  `TILR_SOCK`, the server socket variable, `SSH_AUTH_SOCK` from agent forwarding.
- No login wrappers, no shims: the login shell starts as `-<shell>` directly and reads the user's
  rc files only if the profile grants them.
- Grants that would hand back the server are refused at create, `create pane: sandbox: <path>
  would expose <what>`: any grant covering the state directory (the token), the runtime
  directory (sockets), or, for `read_write`, the configuration directory (the profile itself).
  Landlock grants by allow list only, so a grant of `~` cannot carve these out (Seatbelt could, D10
  adds such denies as defense in depth); refusing on both platforms is the fail-closed answer (so `read_only = ["~"]` is refused when the state
  directory is under home, which it is by default).

### D7. Fail closed
- No `sandbox` feature → `pane-profiles` refuses (already).
- Applying fails (Landlock error, seccomp error, `sandbox-exec` missing or refusing the profile, a grant path that does not exist — skipped with a
  log line, not an error, as a missing directory grants nothing): if the restriction itself
  cannot be installed the child exits before `execve` with a status the pane process reports as
  `create pane: sandbox: <reason>`; the program never runs unrestricted.
- The server never falls back to an unrestricted pane, also not after a reload into a build whose
  kernel/OS check fails: the successor's `state-formats` check already covers format; a pane whose
  restriction is in the kernel stays restricted regardless.

### D8. Temporary directory and cleanup
Each sandboxed pane gets `<runtime dir>/sandbox/<pane id>/` (0700) as `TMPDIR`, removed when the
pane ends; at start the server removes entries there whose pane id is not a running pane (as it
sweeps stale state files). On macOS the directory also holds the generated profile (D10); there
are no other artifacts.

### D9. Protocol
`PaneInfo.sandboxed = 13` (bool). Feature `"sandbox"` appended after `"exec"` in
`HelloAck.features` when the host passes the start-up check (Linux: Landlock ABI ≥ 4 and seccomp
available; macOS: the start-up check of D10). Clients show a badge; `tilr new --profile` needs nothing new (the
server refuses).

### D10. macOS mechanism: Seatbelt with a generated deny-default profile (owner, 2026-11-17)

Decided by the owner on 2026-11-17 after the probe; supersedes the 2026-10-18 decision (App
Sandbox; Seatbelt rejected as deprecated).

**Probe (macOS 26.6.2, arm64), the basis of the decision:**
- Cause of the App Sandbox failure: `/System/Library/Sandbox/Profiles/application.sb` lines
  952-958 allow read/write on `/dev/ttys*` but `file-ioctl` only on `/dev/ptmx`; line 817 gives
  `/dev/tty` read/write only. Inside App Sandbox `tcsetattr`, `TIOCSWINSZ`, `TIOCGETD`,
  `tcgetpgrp`/`tcsetpgrp` and `TIOCSCTTY` return `EPERM` on the slave; `/bin/zsh -i` with job
  control prints `can't set tty pgrp` and hangs; `less` hangs; `vim` stays in cooked mode.
- Alternatives rejected:
  - A pty opened inside the App Sandbox: its slave fails the same way; only the master works.
  - App Sandbox plus `com.apple.security.temporary-exception.sbpl` with `(allow file-ioctl
    (literal "/dev/tty") (regex #"^/dev/ttys[0-9]+$"))`: raw mode, job control, vim and less work,
    but a script created inside is quarantined (`./t.sh` → `bad interpreter: Operation not
    permitted`; `sh t.sh` works); a signed helper `.app` per profile is needed (clang and codesign
    on the user's machine; child binaries need `app-sandbox` + `inherit` or no entitlements, else
    SIGTRAP); `ps` is denied; containers accumulate; notarization acceptance untested.
  - An interposer (a `DYLD_INSERT_LIBRARIES` shim forwarding `tcsetattr` to the pane process
    outside): macOS strips `DYLD_*` for platform binaries (`/bin/zsh`, `/usr/bin/vim`), and
    `tcsetpgrp` cannot be done from a process outside the session.
- Seatbelt with a deny-default profile: the whole tty ioctl matrix passes; `/bin/zsh -i` job
  control works (`sleep 30 &`, `jobs`); `stty raw; stty sane` works; `/usr/bin/vim` and `less`
  start and quit cleanly; a script created inside runs as `./t.sh` (no `com.apple.quarantine`,
  only `com.apple.provenance`); a write outside the granted directory and an outbound TCP connect
  are denied (`EPERM`). No helper app, no signing.

**Accepted risk.** `sandbox-exec(1)` and `sandbox_init(3)` are marked deprecated by Apple since
macOS 10.8 and the profile language (SBPL) is undocumented; the kernel mechanism stays (App
Sandbox and system services are built on it; Chromium, Claude Code, Codex CLI and Bazel rely on
it). Consequences, as requirements and tasks: the `sandbox::*` tests run on every supported macOS
version in CI (a matrix, not only `macos-15`), and an unavailable `sandbox-exec` or a profile
that fails to generate or apply fails closed (no `sandbox` feature, or `create pane: sandbox: …`).

**Where it is applied.** The pane process keeps running unrestricted (pty master, screen model,
reload). In `pty::spawn_shell`, for a sandboxed pane, the child (after `setsid` and `TIOCSCTTY` in
`pre_exec`, as today) execs `/usr/bin/sandbox-exec -f <profile> -D <params>… -- <tilrd exe>
sandbox-shell --ready-fd N --arg0 <arg0> -- <program argv>`. `sandbox-exec` applies the profile
and execs the trampoline `tilrd sandbox-shell` inside the sandbox, which checks it is sandboxed
(`sandbox_check` on its own pid), writes one byte to the close-on-exec ready pipe and execs the
program with the right `argv[0]` (`-zsh` for a login shell, which `sandbox-exec` alone cannot set).
The pane process waits for that byte: none before the pipe closes = the sandbox was not applied,
the pane reports `create pane: sandbox: <sandbox-exec's stderr>`. The pid, session and controlling
terminal are the child's throughout (each step is an `exec`). Rejected: `sandbox_init` in
`pre_exec` — `pre_exec` runs after `fork` in a multithreaded process and may only use
async-signal-safe calls, which `sandbox_init` (profile compilation, allocation) is not; and
`sandbox_init_with_parameters` is private API. The profile is written to
`<runtime dir>/sandbox/<pane id>/profile.sb` (0600) and kept for the pane's life for diagnosis.

**Reload.** A reload re-execs the pane process with its unchanged argv (`tilrd pane --id …`); the
shell and everything under it keep the Seatbelt sandbox (it is attached to their processes by the
kernel and inherited), and the pane process is not sandboxed, so its exec is unaffected. Nothing
new crosses the reload; `pane-state` stays 3. The trampoline's executable path may change with an
update; it is used only at spawn.

**Profile generation.** tilrd writes the SBPL text itself; the text contains only fixed rules
and generated parameter names. Every path from the profile, the environment or the file system
is passed as a `-D NAME=VALUE` parameter and referenced as `(param "NAME")` (names `RO_<n>`,
`RW_<n>`, `EX_<n>`, `DENY_<n>`); no path is ever interpolated into SBPL. Paths with NUL or newline
are refused at profile validation. The rules:

- Always (the set the probe found necessary, a requirement of generation, not an exhaustive
  list): `(version 1) (deny default)`; `(allow file-read* (literal "/"))` (without it every program
  aborts); `(allow file-read-metadata)`; `(allow file-read-data (literal "/dev"))` (for
  `ttyname`); `(allow file-read* file-write* file-ioctl (literal "/dev/tty") (literal "/dev/ptmx")
  (regex #"^/dev/ttys[0-9]+$"))`; `(allow pseudo-tty)`; `(allow process-fork)`; `(allow
  signal (target same-sandbox))`; `(allow process-info* (target same-sandbox))`; `(allow
  sysctl-read)`; `(allow system-sched)` (else zsh prints `nice(5) failed` on `&`); `(allow
  mach-lookup (global-name "com.apple.system.opendirectoryd.libinfo"))`; read on `/usr`, `/bin`,
  `/sbin`, `/System`, `/Library/Apple`, `/private/etc`, the timezone database, `/dev/urandom`,
  `/dev/random`, `/dev/fd`, `/dev/dtracehelper`; read and write on `/dev/null` and the pane's
  `TMPDIR` (D8); `process-exec` on the system program directories (`/usr/bin`, `/bin`,
  `/usr/sbin`, `/sbin`, `/usr/libexec`) and on the trampoline.
- `read_only` → `(allow file-read* (subpath (param "RO_n")))` (a file: `literal`), plus
  `process-exec` under it (the same "read and execute" as Linux D5).
- `read_write` → `(allow file-read* file-write* (subpath (param "RW_n")))` plus `process-exec`
  under it, so scripts and binaries made there run.
- `executables` → resolved when the pane is created: a name through the pane's `PATH`, a path as
  written; both the path as found and its symlink-resolved absolute path get `process-exec` and
  `file-read*` (`literal`). Their dynamic libraries: tilrd reads the Mach-O load commands
  (`LC_LOAD_DYLIB`, `LC_LOAD_WEAK_DYLIB`, `LC_REEXPORT_DYLIB`, resolving `@rpath`,
  `@loader_path`, `@executable_path`) recursively and grants `file-read*` on each library outside
  the always-readable system paths (libraries in the dyld shared cache need nothing). A script in
  `executables` gets its `#!` interpreter resolved the same way (one level; `/usr/bin/env NAME`
  resolves `NAME` through `PATH`). The program the pane runs (command `argv[0]` or login shell) is
  added to `executables`.
- `network`: `none` → no `network*` rule; `client` → `(allow network-outbound (remote ip))`,
  `(allow system-socket)` and DNS (`(allow mach-lookup (global-name "com.apple.dnssd.service"))`,
  `(allow network-outbound (literal "/private/var/run/mDNSResponder"))`); `server` → `client` plus
  `(allow network-bind (local ip))` and `(allow network-inbound (local ip))`.
- Unix sockets: no `network-outbound` rule for any other Unix socket path is ever generated, so
  connecting to tilrd's own sockets (server, host-events, agent) or any other same-user socket is
  denied by `(deny default)` — the macOS counterpart of Linux D5's `AF_UNIX` rule. As defense in
  depth the profile ends with `(deny file-read* file-write* network-outbound (subpath (param
  "DENY_n")))` for the state and runtime directories (later rules win in SBPL), in addition to D6's
  refusal of grants that cover them.
- `env`: handled outside the profile, by the environment built in D6 and passed to
  `sandbox-exec`'s child (Seatbelt does not filter variables).

**Start-up check** (`sandbox` feature on macOS): `/usr/bin/sandbox-exec` is executable and a
fixed probe profile applies to `/usr/bin/true` within 2 s.

**Cleanup.** Only the per-pane directory of D8 (profile and `TMPDIR`); there are no bundles or
containers.

## Reload and reconnect

Linux: the Landlock domain and seccomp filter are on the program's process tree; the pane
process is not restricted and execs its successor as today; the snapshot in `PaneState` tells
the successor the pane is sandboxed (`PaneInfo.sandboxed`, the temporary directory). A program
started in the pane after the reload is restricted because it descends from the restricted shell.
macOS: the same holds for the Seatbelt sandbox (D10). Clients reconnect as after any reload and see the same `PaneInfo`.

## Backpressure

Nothing new on the wire beyond one bool per `PaneInfo`.

## Risks / Trade-offs

- macOS: `sandbox-exec`/`sandbox_init` are deprecated and SBPL is undocumented (D10, accepted by
  the owner); a macOS release that changes them is caught by the CI matrix and fails closed.

- Refusing `AF_UNIX` sockets breaks programs that need a Unix socket outside (D-Bus, an IDE
  bridge). Listed in README; a later grant key can name allowed socket paths if Landlock gains a
  connect right for them.
- `client` on Linux does not restrict UDP (only TCP listening is refused).
- Kernels between 6.7 and 6.12 lack signal scoping: a sandboxed process can signal the user's
  other processes (not the server's ptrace, which Landlock blocks from ABI 1). Open question.
- The refusal of `~`-wide grants (D6) is strict; users must grant subdirectories.

## Open Questions

- Minimum Linux ABI for `sandbox`: 4 (kernel 6.7, signal scoping only when available) or 6
  (6.12, signals and abstract sockets always scoped). Rec.: 4, log a warning per sandboxed pane
  without scoping, revisit when Ubuntu LTS ships 6.12.
- D6: refuse grants covering the state/runtime/config directories, vs moving the token out of
  home. Rec.: refuse (no layout change; fail closed).
- D5: `AF_UNIX` refused always, vs only with `network = "none"`. Rec.: always — the server's
  socket is the escape regardless of the network setting.
- D5 is a deviation from the owner's "Linux: Landlock, unchanged": seccomp is added because
  Landlock does not govern `connect` to named Unix sockets (tilrd's own sockets would be an
  escape) and has no UDP rules. Rec.: confirm Landlock + seccomp.
- D10 macOS versions in the CI matrix: every macOS version tilrd supports (GitHub runners
  `macos-14`, `macos-15`, `macos-26` today). Rec.: all three on the merge queue and nightly;
  pull requests keep the Linux-only quick tier.
- Relation to `read-only-viewer` / `multi-user-sessions`: a "sandbox-only" token kind was listed
  for `read-only-viewer`'s token kinds; the bus rights of `bus-acl-biscuit` give Tilr's runtimes
  "create pane with profile X only" without a new token kind. Rec.: no TCP token kind now.
