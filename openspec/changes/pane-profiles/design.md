# Design: Launch profiles for panes

## Context

At 14388b16 a pane is described entirely by its `CreatePane` (`cols`, `rows`, `command`, `cwd`);
the supervisor clamps the size, picks the directory with `start_dir` (the requested one if it
is a directory, else home), clones `pane_env` (shims, socket variables, the agent socket) and
runs `tilrd pane --id <uuid> [--cols --rows] --cwd <dir> -- <command…>`
(`supervisor.rs` `create_pane`, `start_pane_process`); the pane process opens the pty and runs
the program (`pty.rs` `spawn_shell`; `pre_exec` does `setsid` and `TIOCSCTTY`). A failure is an
`Error` `create pane: …` (no `request_id`: `Error` has none; `tilr`'s `conn.rs` `answer()`
treats any `Error` while waiting as the answer). `server.toml` is found in `$TILR_CONFIG_DIR`,
`$XDG_CONFIG_HOME/tilr` or `~/.config/tilr` (or named by `--config` / `TILR_CONFIG`), read at
start, strict (`deny_unknown_fields`). The supervisor hands each pane over on reload as
`PaneState{info: PaneInfo, …}` in `SupervisorState` (format 2, `reload.rs` `STATE_FORMAT`).

The prototype (`runtime-tmp`, `docs/how-it-works.md` §12, `sandbox/rs/src/lib.rs`) has the
sandbox spec this change adopts: `read_only`, `read_write`, `network` (`none` default |
`client` | `server`), `env` (inherited names), `executables` (PATH names or paths, plus their
libraries, granted read-only), `path`; `~` is the real home, relative paths resolve against the
file's directory, a directory grants its subtree; a profile file is read on every
`term.create`.

Owner's decisions of 2026-11-17 (fixed): named profiles; one file per profile at
`profiles/<name>.toml` in the config directory; the sandbox part optional; usable by hand and by
Tilr itself; a pane without a profile unchanged.

## Goals / Non-goals

Goals: the profile file format and its validation; resolving a name safely; combining a profile
with the request; listing profiles; refusing sandboxed profiles until `sandboxed-pane`;
keeping the applied profile across a reload; the `tilr` commands.

Non-goals: enforcing the sandbox; bus access to profiles; editing profiles remotely; profile
inheritance.

## Decisions

### D1. Where profiles live
`<config dir>/profiles/<name>.toml`, where `<config dir>` is the directory of the `server.toml`
the server uses: `$TILR_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tilr`, else `~/.config/tilr`; with
`--config PATH` (or `TILR_CONFIG`), the directory of `PATH`. A missing `profiles` directory
means no profiles. Rejected: a `[profiles.<name>]` section in `server.toml` (owner); a separate
`TILR_PROFILES_DIR` (one more variable for tests and services to keep in step; the config
directory already has one). Tying the directory to `--config` keeps a test's or a service's
whole configuration in one place, as systemd's drop-ins sit next to their unit.

### D2. Names, never paths
A name SHALL match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. Anything else (empty is "no
profile", so it is not an error) is refused before any file system access: `create pane: bad
profile name: <name>`. The name has no `/`, cannot be `.` or `..` and cannot start with `.`
(no hidden files, no editor backups such as `.x.toml.swp` named by accident); the server joins
`profiles/` + name + `.toml` itself. The file may be a symlink (the user's own config, as with
dotfile managers); it must resolve to a regular file. Rejected: accepting a path from the client
(the prototype's `sandbox: ./file.yaml` form) — a client on the TCP port must not make the
server read arbitrary files.

Ownership check: the file (after symlinks) and the `profiles` directory MUST be owned by the
server's user and not writable by group or others, else the profile is invalid
(`… is writable by others`). A sandbox profile is a security boundary: a file anyone can edit
turns it off. This is ssh's `StrictModes` for `authorized_keys`. Listed under Open Questions.

### D3. Format
TOML, strict like `server.toml` (`deny_unknown_fields` at every level):

```toml
description = "Claude Code in the tilr repo, no network but the API"   # optional
command = ["claude"]            # argv; absent or [] = the login shell
cwd = "~/projects/tilr"         # optional
[env]                           # variables set in the pane
CLAUDE_CONFIG_DIR = "~/.claude-agent"
[sandbox]                       # optional; its presence makes the profile sandboxed
read_only = ["~/projects/tilr", "/opt/homebrew", "~/.gitconfig"]
read_write = ["~/work/agents/001"]
network = "client"              # none (default) | client | server
executables = ["zsh", "git", "claude"]
env = ["LANG", "LC_ALL", "USER", "LOGNAME"]
```

- `command`: array of strings, each non-empty, no NUL. Not a shell string (no word splitting).
- `cwd`, and every path in `[sandbox]`: absolute, `~`, or `~/…`; `~` is the server user's home
  from the password database. Relative paths are an error. **Deviation from the prototype**
  (relative = against the file's directory): profiles live in the config directory, not in a
  project, so "relative to the file" would mean `~/.config/tilr/profiles/…`, never what was
  meant. `.` and `..` components are folded lexically, as in the prototype. Paths in `[env]`
  values are not expanded except a leading `~/` (so `HOME`-relative settings work); other
  values are passed as written.
- `[env]`: string to string. A name must be non-empty and contain no `=` or NUL; names starting
  with `TILR_` are refused (they belong to the server: `TILR_PANE`, `TILR_SHIMS`, the socket
  variables). Applied after the server's environment and before the terminal variables
  (`TERM`, `COLORTERM`, `SHELL`, `TILR_PANE`), which the server sets last, as today.
- `[sandbox]`: semantics as the prototype's spec — a directory grants its subtree;
  `read_write` implies read; `network` `none` | `client` (outgoing) | `server` (outgoing and
  incoming); `executables` are PATH names (looked up in the pane's `PATH` when the pane is
  created) or absolute paths; `env` names the server-environment variables a sandboxed pane
  keeps (allowlist; everything else is dropped). **Deviation**: no `path` key — `PATH` is set
  through `[env]` like any variable, or inherited when listed in `env`; when neither, change
  `sandboxed-pane` builds it from the `executables` directories and the system directories (the
  prototype's default). Unknown `network` values are a parse error.
- Lists may be empty; an empty `[sandbox]` table is a sandbox with no grants beyond what
  `sandboxed-pane` always allows.

### D4. Read on every create, snapshotted per pane
The file is read and validated on each `CreatePane` that names it (and each `ListProfiles`),
never cached: an edit applies to the next pane, as in the prototype's `term.create`. The
resolved result (absolute paths, final argv, cwd, env additions, sandbox) is the pane's
**snapshot**; the pane keeps it for its life and across reloads (D7), and never re-reads the
file. Rejected: reading all profiles at start (needs a restart or a watch; the prototype's
watch logic is the most complex part of its host).

### D5. Request fields override the profile
- `command`: a non-empty `CreatePane.command` replaces the profile's `command`; empty uses the
  profile's; neither = the login shell.
- `cwd`: a `CreatePane.cwd` that names an existing directory wins; else the profile's `cwd` if it
  is an existing directory; else home (today's fallback).
- No request field can change `[env]` or `[sandbox]`.

Security check for "a sandboxed profile forbids overriding `command`": not needed. The sandbox
restricts whatever runs in the pane, so a different command cannot do more than the profile's
command could (it could itself exec anything the sandbox lets it). Today every client that may
send `CreatePane` may also start an unrestricted pane, so an override gives it nothing. When a
restricted principal appears (a bus runtime allowed to create only profile `X`, change
`bus-panes`), what it may override is a right in its token, not a property of the profile.
`sandboxed-pane` grants the command actually run (`executables`) read access; that grant is
read-only and the client could name the same file in a profile anyway.

`tilr new --profile NAME` sends `cwd` only when `--cwd` is given, else the profile's directory
would always lose to the current directory `tilr new` sends today.

### D6. Protocol
- `CreatePane.profile = 6` (string; empty = none).
- `PaneInfo.profile = 12` (string; the name at creation; empty = none).
- `ListProfiles { uint64 request_id = 1; }` = `ClientMessage` 32.
- `ProfileList { repeated ProfileInfo profiles = 1; uint64 request_id = 2; }` = `ServerMessage` 25,
  sorted by name; `ProfileInfo { string name = 1; string description = 2; bool sandboxed = 3;
  string error = 4; }`. Files in `profiles/` whose stem is not a valid name, or that do not end
  in `.toml`, are skipped. A file that fails D2/D3 is listed with its `error` (the same text
  `CreatePane` would give) and `sandboxed` as far as it parsed.
- Errors on `CreatePane` (each an `Error`, no pane, connection stays open):
  `create pane: bad profile name: <name>`;
  `create pane: no such profile: <name>`;
  `create pane: profile <name>: <file>: <reason>` (parse error, unknown key with its name,
  relative path, ownership);
  `create pane: profile <name> asks for a sandbox, which this server cannot enforce`.
  A prefix the clients already know (`create pane: `) keeps `tilr`'s handling unchanged.

### D7. Reload
`PaneState` gains `ProfileSnapshot profile = 6` (name, argv, cwd, env additions, and the
resolved sandbox as a nested message), and `PaneInfo.profile` travels in `PaneState.info`.
`SupervisorState.format` goes from 2 to 3 so `tilrd state-formats` stays honest: an older
successor would drop the snapshot silently. The pane process gets what it needs from its
command line and environment as today; `sandboxed-pane` decides how the sandbox part reaches it.
A client reconnecting after a reload sees the same `PaneInfo.profile`.

### D8. No feature flag
The repo rule allows a `HelloAck.features` entry only for a capability a server can be run
without. Profiles cannot be turned off: a server with no `profiles` directory still answers
`ListProfiles` (empty) and refuses unknown names. So no `profiles` flag. Sandboxing can be
absent (old kernel, unsigned build), so `sandboxed-pane` adds a `sandbox` flag.

Old servers: an older pre-release tilrd ignores `CreatePane.profile` and starts an
unrestricted pane with the request's command. `tilr` checks that `PaneCreated.pane.profile`
equals the name it sent; if not, it closes the pane at once and exits 1 with
`the server does not know profiles; update it`. The command may already have started. This is
acceptable before the first public release (`tilr` and `tilrd` ship in one archive; bus clients
run inside tilrd); listed under Risks and Open Questions.

## Relation to other changes

- `sandboxed-pane` enforces `[sandbox]` and lifts the refusal of D6 where the `sandbox` feature
  is listed.
- `bus-panes` lets runtimes create panes by profile name; which profiles a runtime may use and
  whether it may override `command`/`cwd` (D5) are rights in its Biscuit token, not profile keys.
- `read-only-viewer`: a read-only connection may not `CreatePane`, so it cannot use profiles;
  `ListProfiles` is not on its allow list either (profile names and descriptions are the owner's
  configuration). That change's allow list stays as designed.
- `multi-user-sessions`: in a system tilrd, the configuration directory is `/etc/tilr`; per-user
  profiles would live in the user's own config directory, read by the per-user helper (its Q6),
  never by root. Not designed here; D1 is written for the per-user server.

## Reload and reconnect

See D7. The `profiles` directory is not state; a profile edited or removed after a pane started
does not affect that pane, before or after a reload.

## Backpressure

`ProfileList` is one small message per request on the interactive lane; its size is bounded by
the number of files (the server lists at most 1000 profiles and logs that more were skipped)
and each `description`/`error` is cut at 1 KiB.

## Risks / Trade-offs

- Old servers ignore `profile` (D8): a sandboxed profile's command could start unrestricted on a
  pre-release server older than this change. Mitigated by `tilr`'s check; bus runtimes are not
  affected (same binary).
- The ownership check (D2) can surprise users with a group-writable home setup; the error names
  the file and the mode.
- No cache: each create reads one small file; negligible next to forking a pane process.

## Open Questions

- D2 ownership check (`StrictModes`-like). Rec.: keep it (ssh does the same for
  `authorized_keys`; a sandbox profile must not be editable by others).
- D1: profiles next to `--config`'s file rather than always in the default config directory.
  Rec.: next to it (one directory per configuration, like systemd drop-ins).
- D8: no `profiles` feature flag, `tilr` checks `PaneInfo.profile` after the fact. Rec.: keep
  the rule (no flag); the window exists only against pre-release servers.
- D3: `description` key (not in the owner's list). Rec.: keep it, optional; the Tilr app needs
  something to show in a picker.
