# Proposal: One bus for everything (Phase 7)

Зависит от: -

## Why

Owner's decisions of 2026-11-17 (session TILR-892), recorded in two rounds.

First round: panes get named launch profiles (`profiles/<name>.toml`); the sandboxed pane is
done now (Seatbelt on macOS after the probe, Landlock on Linux); the component bus of the
prototype `runtime-tmp` moves into this repo with Biscuit rights. They were written as seven
changes (`pane-profiles`, `sandboxed-pane`, `component-bus`, `bus-acl-biscuit`,
`bus-runtime-sdks`, `bus-gateway`, `bus-panes`).

Second round (this change), fixed by the owner and not reopened here:

- The bus is the one transport for everything: Deno and Python components, Tilr Web (JS
  plugins in the app's window), the terminal and tilrd's own core. Any participant can talk
  to any other through the broker in tilrd: a JS plugin in Tilr Web calls code in Deno or
  Python and back; a Python script drives a terminal; tilrd calls a method a plugin provides.
- `tilr.bus.v1` has calls (Call/Reply), events (subjects, exact and wildcard subscriptions,
  at-most-once to the subscribers connected now, nothing stored) and one-way streams (a call
  answered by Chunk* then End or Error, cancelled by the caller, with backpressure).
- Tilr Web is a full participant over WebSocket with a Biscuit token in Hello: it calls,
  subscribes and provides methods. This revokes "a participant is only an instance tilrd
  started" and "attach instances are removed" of `component-bus`: outside participants exist,
  with a token and its rights only.
- The terminal moves onto the bus in this phase: `crates/tilr-proto/proto/tilr.proto` is
  removed whole; every message becomes a method, event or stream (`pane.*`, `server.*`, not
  `term.*`); tilrd's core is a native participant; `tilr`, `tilr-driver`, the tests and itests
  move to the bus in the same change.
- Instances survive tilrd's reload; panes outlive the participant that made them; an instance
  that loses its connection is restarted. Bus config is its own `bus.toml`; the rights key is
  `rights`; no `bus` or `profiles` feature flag.
- Rights: `Call.credential` is a bearer; tokens expire after 24 h and are renewed; a root key
  rotation ends every token at once (no `root_key_id`); authorization is limited to 1 ms and
  cached; no "sandbox only" token kind (a right to create panes with profile X); `pane_access`
  defaults to `own`.
- Profiles: a request's command overrides the profile's and the profile's sandbox still
  applies; profile files are checked like ssh's `StrictModes`; `profiles_dir` in
  `server.toml` (default `profiles/` next to it); `description` optional.
- Sandbox: Linux Landlock + seccomp (Landlock ABI ≥ 4; signal scoping where ABI 6, else a
  warning per pane); `network = "client"` leaves UDP open (documented limit); `AF_UNIX` always
  refused; grants covering the state, runtime or config directory refused; CI on macOS 14, 15
  and 26 in the merge queue and nightly.
- SDKs: Deno 2.x, Python 3.12 and browser JS (the Deno package over WebSocket), covering
  call, provide, emit/on and stream; not published before the release that makes the protocol
  public.
- One change, one task, one branch, one PR for the whole phase; the HTTP gateway is deferred
  (Tilr Web reaches the bus over WebSocket, scripts through `tilr bus call`); Tilr.app (another
  repository) moves after, on a new minor release line cut on the owner's word, and no
  `tilr.proto` → bus adapter is planned.

## What Changes

- **BREAKING** (client protocol): `tilr.proto` (package `tilr.v1`), the `tilr-proto` crate and
  `PROTOCOL_VERSION` of `tilr.v1` are removed. The TCP port and the Unix socket speak
  `tilr.bus.v1` over WebSocket (subprotocol `tilr.bus.v1`); a client without that
  subprotocol gets one `tilr.v1` `Error` saying the server speaks `tilr.bus.v1`, then the
  connection closes. Migration: clients move to the bus (this repo's in this change, Tilr.app
  on its own release line, design M2). The full mapping of every `ClientMessage` and
  `ServerMessage` is design T2.
- New crate `crates/tilr-bus`: `proto/bus.proto` (package `tilr.bus.v1`), the payload schemas
  of the core's methods (`proto/tilr/{pane,server,fs,exec,clipboard,host,agent}/v1/*.proto`,
  holding the frame, input and pane types that were in `tilr.proto`), framing for WebSocket and
  stream sockets, and the async Rust client used by `tilr`, `tilr-driver`, `tilrd ps` and the
  tests.
- Broker in tilrd (`crates/tilrd/src/bus/`): calls routed by participant or method, events
  with subjects and wildcard subscriptions (at-most-once, no history), one-way streams with
  per-stream and per-connection byte credits, interactive and bulk lanes, bounded queues,
  deadlines and cancellation, ordered delivery per caller and provider.
- Participants: tilrd's core (`tilrd`), instances spawned from `bus.toml` (fd or instance
  socket), and outside participants over WebSocket on the TCP port or the Unix socket (Tilr
  Web, `tilr`, scripts). Instances are supervised, restarted when their connection is lost, and
  handed over across tilrd's reload.
- The terminal on the bus: `pane.create|list|close|resize|input|read|applied|input_finalize`,
  the stream `pane.output` (the screen frames of today, full then diffs, skipping to the latest
  screen for a lagging reader), events `pane.created|changed|title|bell|command_finished|
  copied|exited`; `server.info|reload|update|command`; `fs.*`, `exec.run` (stream) and
  `exec.input|signal`, `clipboard.put|get`; methods clients provide and tilrd calls:
  `host.open`, `ssh_agent.request|close`.
- Rights: Biscuit tokens (`biscuit-auth`) for every participant but `owner` (a Unix-socket
  connection of the user, or a TCP connection with the access token): `call`, `provide`,
  `emit`, `subscribe`, `control`, `pane` rights from `[instances.<name>.rights]` or minted with
  `tilr bus token`; 24 h expiry with renewal; revocation; key rotation; 1 ms limit with
  caches; audit lines. New close code 4003 `credential` (token expired, revoked or replaced).
- Tilr Web as a participant: Origin checked on the WebSocket handshake, a minted token bound to
  its origin, provided methods (`ui.notify` and the like) routed to the last active window,
  `unavailable` when no window provides them.
- Launch profiles (`profiles_dir`, default `profiles/` next to `server.toml`) used by
  `pane.create {profile}`, listed by `pane.profiles`; `tilr new --profile`, `tilr profiles`.
- Sandboxed panes from a profile's `[sandbox]`: Landlock + seccomp on Linux, Seatbelt with a
  generated deny-default profile on macOS; a normal terminal inside; fail closed; survives a
  reload. `server.info.features` lists `sandbox` where it can be enforced.
- SDKs `sdk/deno` (also loaded by browser plugins) and `sdk/python` with call, provide,
  emit/on, stream; CI job `sdk`.
- `tilr bus call|ls|start|stop|reload|reload-config|token|revoke|rotate-key|sub`.
- Reload state: `supervisor-state 3` (profile snapshot, creator), new `bus-state 1`
  (instances, their connections, tokens); the internal schemas move from `tilr.proto` to
  `crates/tilrd/proto/internal.proto`.
- Protocol version policy: `tilr.bus.v1` starts at protocol version 1 and stays 1 until the
  first public release (README "Protocol"); feature flags (`server.info.features`) stay only for
  what a server can be run without (`clipboard`, `files`, `host-events`, `agent`, `exec`,
  `sandbox`); no `bus` or `profiles` flag.

## Capabilities

### New Capabilities
- `component-bus`: the protocol `tilr.bus.v1`, broker, calls, events, streams, backpressure,
  participants and transports, `bus.toml`, instances, supervision, control, `tilr bus`, reload.
- `bus-acl`: Biscuit tokens and rights, minting, renewal, revocation, rotation, limits, audit.
- `bus-web`: Tilr Web (browser pages) as bus participants.
- `bus-sdks`: the Deno/browser and Python libraries.
- `bus-terminal`: the terminal on the bus (`pane.*`, `server.*`, `fs.*`, `exec.*`,
  `clipboard.*`, `host.open`, `ssh_agent.*`), pane rights, the removal of `tilr.proto`.
- `pane-profiles`: profile files, names, format, resolution, listing, reload.
- `pane-sandbox`: what a sandboxed pane may and may not do, how it behaves, fail closed.

### Modified Capabilities
- `protocol`: the transport and handshake are the bus's (`tilr.bus.v1` over WebSocket);
  requirements tied to `ClientMessage` / `ServerMessage` are replaced.
- `panes`: "Creating a pane", "The directory a pane starts in" (profiles, `pane.create`).
- `tilr-cli`: "Command set" (`profiles`, `bus`), "new" (`--profile`).
- `reload`: "state-formats names what this executable reads".

The other main specs that name `tilr.proto` messages are rewritten to the bus names of design
T2 within this change (tasks 11.2); their behavior does not change.

## Non-goals

- The HTTP gateway (`tilrd gateway`, `POST /call/...`): deferred (Tilr Web uses WebSocket,
  scripts `tilr bus call`); its draft is not carried over.
- An adapter from `tilr.proto` to the bus, or serving both protocols at once.
- Moving Tilr.app (another repository) in this change.
- Two-way streams or channels; client-to-server streams (uploads are sequences of calls).
- A direct channel between participants bypassing the broker (the protocol leaves room).
- Event history, replay, durable subscriptions or queue groups (JetStream-like features).
- Publishing the SDKs to JSR, PyPI or npm.
- Biscuit for the TCP access token itself, third-party blocks, proof of possession.
- Sandboxing bus instances; sandboxing `exec.run` or the files methods.
- App Sandbox, VMs, Endpoint Security, a `DYLD_INSERT_LIBRARIES` interposer; tilrd in Docker.
- Network isolation beyond `none | client | server`; per-pane loopback aliases.
- Moving panes into a separate `tilr-panes` component.
- `close_with_creator` for panes (possible later).

## Impact

Code: new `crates/tilr-bus`; `crates/tilr-proto` removed; new `crates/tilrd/src/bus/`
(`broker`, `events`, `streams`, `runner`, `config`, `control`, `acl`, `core`), new
`crates/tilrd/src/profile.rs`, `crates/tilrd/src/sandbox/`; `crates/tilrd/src/{supervisor,
pane,pty,outbox,link,reload,local,ps,files,exec,clipboard,host,agent,main,config}.rs`;
`crates/tilr-screen` (payload types from `tilr.pane.v1`); `crates/tilr/src/`,
`crates/tilr-driver/src/`; every test in `crates/*/tests/`; `itests/`; new `sdk/deno/`,
`sdk/python/`, `scripts/gen-bus-sdk.sh`, `scripts/sdk-check.sh`, `scripts/install-tools.sh`
(deno, uv); `.github/workflows/ci.yml` (job `sdk`, macOS 14/15/26 sandbox matrix);
`deny.toml` if `biscuit-auth` or a Landlock crate needs a license; README (Protocol rewritten,
new "The bus", "Launch profiles", "Sandboxed panes"), USE_CASES.md, itests/USE_CASES.md,
AGENTS.md (prerequisites, checks, the `tilr-proto` line).

Clients outside this repo: Tilr.app's `server` plugin and tilr-mac speak `tilr.v1` and cannot
talk to a server of this change; they stay on the current release line (design M2).

Reload: instances, their connections and tokens cross the reload (`bus-state 1`); panes keep
their profile snapshot and creator (`supervisor-state 3`); client and outside-participant
connections end and reconnect, as clients' do today. A reload from a build before this change
into one after it is refused by the `state-formats` check (Open Questions). Update from the
previous release: no `bus.toml`, no `profiles/`, a root key made on first start; the update
itself is a release of a new minor line, cut on the owner's word once Tilr.app speaks the bus.

Release-gate rows of `openspec/ROADMAP.md` that name `tilr.proto` (`tilr-mac-rename-
compatibility`) concern the current release line and are not changed by this change.
