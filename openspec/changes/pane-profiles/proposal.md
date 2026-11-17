# Proposal: Launch profiles for panes

Зависит от: -

## Why

The owner decided on 2026-11-17 (session TILR-892) that a pane can be created with a named
launch profile, that a profile is one TOML file per profile in the tilr config directory
(`profiles/<name>.toml`, not a section of `server.toml`), that its sandbox part is optional,
and that profiles are used both by hand (`tilr`) and by Tilr itself (protocol now, runtimes on
the component bus later, change `bus-panes`). A pane without a profile behaves as today. The
sandbox itself is change `sandboxed-pane`; this change gives it its request format.

Checked at 14388b16: `CreatePane` has `cols`, `rows`, `command`, `cwd`, `request_id` only
(`crates/tilr-proto/proto/tilr.proto:124`); the supervisor starts every pane with its own
environment (`supervisor.rs` `create_pane`, `start_pane_process`); the configuration directory is
`$TILR_CONFIG_DIR`, else `$XDG_CONFIG_HOME/tilr`, else `~/.config/tilr` (`config.rs`
`default_path`), and `server.toml` is parsed with `deny_unknown_fields`. Nothing in `crates/`
or README mentions profiles in this sense. The prototype in `runtime-tmp` resolved a sandbox
profile by name from `sandboxes/<name>.yaml` and read it on every `term.create`
(`docs/how-it-works.md` §12); this change takes the per-file, read-on-create model and its
field set.

## What Changes

- A profile is the file `<config dir>/profiles/<name>.toml`, where `<config dir>` is the
  directory of the `server.toml` in use. Keys: `description`, `command` (argv array; absent or
  empty = the login shell), `cwd`, `[env]` (variables set), and an optional `[sandbox]` table
  with `read_only`, `read_write`, `network` (`none` | `client` | `server`), `executables` and
  `env` (names of inherited variables kept). Unknown keys are an error, as in `server.toml`.
- A profile name is 1-64 characters of `[A-Za-z0-9._-]`, starting with a letter or digit; a
  client sends only a name, never a path. The file is read on every `CreatePane` that names it.
- `CreatePane.profile` (field 6) names a profile. Explicit `command` and `cwd` in the request
  win over the profile's (design D5). Unknown, misnamed or unparsable profiles are refused with
  `Error` `create pane: …` and no pane starts.
- A profile with `[sandbox]` is parsed and validated, and in this change **always refused**
  (`create pane: profile <name> asks for a sandbox, which this server cannot enforce`): a pane
  is never started unrestricted in place of a sandboxed one. Change `sandboxed-pane` lifts the
  refusal where it can enforce.
- `PaneInfo.profile` (field 12) carries the profile name (empty: none).
- `ListProfiles{request_id}` (ClientMessage 32) is answered with `ProfileList{profiles,
  request_id}` (ServerMessage 25): each `ProfileInfo` has `name`, `description`, `sandboxed`
  and `error` (non-empty when the file does not parse or validate).
- The pane keeps its profile name and the resolved snapshot it was started with across a reload
  (`PaneState.profile`); `supervisor-state` goes from format 2 to 3.
- `tilr new --profile NAME` and `tilr profiles` (list). With `--profile`, `tilr new` sends
  `cwd` only when `--cwd` is given, so the profile's `cwd` applies.
- Protocol: version 1 stays (policy in `openspec/config.yaml`); all fields and messages are
  additive. No feature flag: a server cannot be run without profiles (design D8).

## Capabilities

### New Capabilities
- `pane-profiles`: profile files, names, format, resolution, listing, reload.

### Modified Capabilities
- `panes`: "Creating a pane" and "The directory a pane starts in" (profile).
- `tilr-cli`: "Command set" (`profiles`) and "new" (`--profile`).
- `reload`: "state-formats names what this executable reads" (`supervisor-state 3`).

## Non-goals

- Enforcing `[sandbox]` (change `sandboxed-pane`).
- Profiles for bus instances, and bus methods that create panes (changes `component-bus`,
  `bus-panes`).
- Editing profiles over the protocol, watching the profiles directory, defaults inherited
  between profiles (`extends`), per-profile access rights.
- Showing the profile in `tilr ls` / `tilr ps` output (their columns are a script contract;
  later, if wanted).
- An `env` field in `CreatePane`.

## Impact

`crates/tilr-proto/proto/tilr.proto` (`CreatePane.profile`, `PaneInfo.profile`,
`ListProfiles`, `ProfileList`, `ProfileInfo`, `PaneState.profile`, `ProfileSnapshot`; additive),
new `crates/tilrd/src/profile.rs`, `crates/tilrd/src/{supervisor,config,reload}.rs`,
`crates/tilr/src/{main,conn}.rs`, README ("The configuration file", "Launch profiles",
"Protocol"), USE_CASES.md.

Reload: the profile name and snapshot are handed to the successor in `PaneState`; the file is
not read again for an existing pane. `supervisor-state` format 3: a reload between a build
before this change and one after it is refused by the `state-formats` check (the running server
keeps running; panes survive a restart only by staying in the old build), as for every earlier
format change. Update from the previous release: a server without profiles directory has none;
nothing else changes.

Not breaking for the wire: additive messages and fields, no PROTOCOL_VERSION bump, no close
code or CDN change. The reload state format changes (see above).
