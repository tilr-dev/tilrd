# Design: One bus for everything (Phase 7)

## Context

### What exists (checked at 14388b16)

- Client protocol: `crates/tilr-proto/proto/tilr.proto`, package `tilr.v1`: `ClientMessage`
  (31 oneof messages, plus the envelope fields `seq`, `input_seq` and the internal
  `input_identity`, `input_probe`, `input_forget`) and `ServerMessage` (24 oneof messages);
  one packet = one binary WebSocket message, flag byte (0 plain, 1 zstd at 256 B or more,
  64 MiB decompression cap; `crates/tilr-proto/src/lib.rs:4`, `:52-60`, `:84`). Served on the
  TCP port (default 127.0.0.1:7890) and on the Unix socket, on every path but `/socks`;
  `/metrics` is HTTP. The same file holds the internal schemas: supervisor ↔ pane process
  (`PaneStatus`, `PaneInputAck`, `PendingInput`, ...) and the reload state (`SupervisorState`,
  `PaneState`, `PaneHandover`).
- **What a client receives as a pane's output is screen frames, not pty bytes.** The pane
  process owns the pty and the screen model (`crates/tilr-screen` on libghostty-vt); at most
  every 8 ms of coalescing it builds a `Frame` from the model (full, or the changed rows),
  stamps `pane_id`, `seq`, `last_input_seq`, `timing`, and encodes it once, zstd included
  (`crates/tilrd/src/pane.rs:1010` `send_frame`, encode at `:1064`), then writes it to the
  supervisor with a `FrameHead {full, last_input_seq}`. A frame carries rows of styled runs,
  cursor, title (when changed), modes (`mouse_tracking`, `alt_screen`), Kitty graphics
  placements and the PNG-reencoded images they use (each once per `(id, generation)` per
  subscriber, all in view again on a full frame), hyperlinks, cell size, `viewport_top`. The
  supervisor forwards the same bytes to every subscriber on the interactive lane
  (`crates/tilrd/src/supervisor.rs:2962` `on_pane_output`); a subscriber whose lane held a
  packet over 250 ms (`crates/tilrd/src/outbox.rs:54` `LAG`) has its diffs skipped and, once
  drained, gets a full frame (`supervisor.rs:3492` `ask_full_frame`) whose `last_input_seq`
  is rewritten to that client's latest echoed seq; a subscriber whose lane holds 1024 packets
  (`outbox.rs:41`) is dropped with 4001. Raw pty bytes never leave the pane process.
- Lanes (`crates/tilrd/src/outbox.rs:1-30`): interactive (frames, answers, acks, pushes)
  first; bulk (`FsData`, `StreamData`/`StreamClose`, `ExecOutput`/`ExecExit`,
  `ClipboardWrite`) bounded at 4 MiB queued per client (`BULK_BYTES`) and 1 MiB written but
  unread (`BULK_WINDOW`, paced by marked WebSocket pings).
- Input: keys, mouse, paste, raw, scroll, resize, clear carry a latency `seq` (echoed in
  `Frame.last_input_seq`) and a delivery `input_seq` per stable `client_id` and pane, applied
  once and acknowledged (`InputAck`) after the pty took the bytes, reported back in
  `HelloAck.applied_input`, finalized with `FinalizeInput` (protocol spec).
- Reload: the supervisor and every pane process exec in place; client connections end and
  clients reconnect; panes, screens, pending input and the listeners cross the exec in
  `server-<pid>.state` (format 2) and `panes/<id>.state` (format 3); `tilrd state-formats`
  guards the successor (`crates/tilrd/src/reload.rs:63-120`).
- Prototype `runtime-tmp` (commit 9fbbc3e per its docs): star broker with Call/Reply, ACL by
  instance-name globs, fd/socket transports, supervision, control methods, Deno/Python
  libraries, a `term` runtime without an emulator; its limits are listed in
  `docs/how-it-works.md` §10, its sandbox experiments in §12.
- First-round drafts (2026-11-17, now folded in here): `pane-profiles`, `sandboxed-pane`,
  `component-bus`, `bus-acl-biscuit`, `bus-runtime-sdks`, `bus-gateway` (deferred, not carried
  over), `bus-panes`.

### Owner's decisions

The proposal lists them (Why). This design implements them and decides what they leave open.
Where a first-round decision is revoked, the decision below says so.

## Goals / Non-goals

Goals: one protocol, `tilr.bus.v1`, for every participant (core, instances, `tilr`, Tilr Web,
scripts); every capability of `tilr.v1` kept, with each guarantee restated on the bus (T3);
least privilege with Biscuit for everyone but the owner; profiles and sandboxed panes; SDKs for
Deno, the browser and Python; one branch that switches everything at once and leaves `main`
green.

Non-goals: see proposal.

## Decisions

Each decision names how proven systems do it (first) and what was rejected.

### Part B. The bus

#### B1. Topology: a star with the broker in tilrd
Every frame goes through the broker; participants do not know each other's addresses. This
is D-Bus (`dbus-daemon`) and NATS core: one router, names instead of addresses, identity set
by the router. The broker lives in tilrd (decision of 2026-10-26) and shares its reload.
tilrd's core is a participant named `tilrd` that lives in the broker's process (no socket, no
encoding on the hop: it gets decoded envelopes and the verified token). Rejected: peer-to-peer
connections brokered once (gRPC with a registry, Consul): every participant would need a
listener and its own authorization; a hot-path direct channel stays possible later as an
additive envelope (`ChannelGrant`).

#### B2. Participants and identities
| participant | connects by | identity (`from`) | authorized by |
|---|---|---|---|
| tilrd's core | in process | `tilrd` | everything (it is the server) |
| instance from `bus.toml` | fd 3 or instance socket | `instance:<name>` | token minted at each start |
| owner | WebSocket on the Unix socket (peer uid), or on TCP with the access token, no credential | `owner` | every right |
| outside participant | WebSocket (TCP or Unix socket) with a credential | `token:<name>` (from the token) | that token |

The identity is set by the broker, never by the sender (`from` in a forwarded call is
overwritten), as D-Bus sets the unique name and NATS the client id. **Revokes** the first
round's "a participant is only an instance tilrd started" (`component-bus` D11): outside
participants exist, but only with a credential and its rights; the owner's ambient access is
today's (`access-control`: Unix socket peer check, TCP access token). `tilr` is an outside
participant like any other: as owner on the Unix socket, or with a token.

#### B3. Transports and framing
- WebSocket (TCP port and Unix socket, every path but `/socks` and `/metrics`), subprotocol
  `tilr.bus.v1` (`Sec-WebSocket-Protocol`, as MQTT's `mqtt` and GraphQL's
  `graphql-transport-ws`). One binary message = flag byte (0 plain, 1 zstd) + one `Envelope`;
  zstd at 256 B or more unless the envelope's payload is already compressed (B9); kept from
  `tilr.v1` because it is proven here and browsers have no other framing. This is how nats.ws
  carries NATS core to browsers.
- Stream sockets (instance fd, instance socket): u32 big-endian length + `Envelope`, no
  compression (local), as the prototype.
- Limits: `max_frame` 16 MiB per envelope after decompression (the prototype's; `tilr.v1`'s
  64 MiB cap is lowered, B9 splits frames); in both directions.
- A handshake without the subprotocol (a `tilr.v1` client: Tilr.app on the old line) is
  accepted at the WebSocket level and answered with one hard-coded `tilr.v1` packet:
  `ServerMessage{error: Error{message: "this server speaks tilr.bus.v1; update the client"}}`,
  then close 1002. No `tilr.v1` schema is kept for it (11 bytes of constant plus the text).
  Rejected: an HTTP 400 at the handshake (the old app shows a bare "cannot connect"); serving
  `tilr.v1` too (owner: no adapter).

#### B4. Envelope
```proto
package tilr.bus.v1;
message Envelope { oneof msg {
  Hello hello = 1; HelloAck hello_ack = 2; Call call = 3; Reply reply = 4; Cancel cancel = 5;
  Event event = 6; Subscribe subscribe = 7; Unsubscribe unsubscribe = 8;
  Chunk chunk = 9; End end = 10; Credit credit = 11; CredentialUpdate credential_update = 12;
  Activate activate = 13; ConnectionError connection_error = 14; } }
message Hello { uint32 protocol_version = 1; string client_name = 2; string client_id = 3;
  string access_token = 4; bytes credential = 5; string instance_token = 6;
  repeated MethodSpec provides = 7; }
message MethodSpec { string method = 1; Kind kind = 2; Lane lane = 3; Select select = 4; }
enum Kind { UNARY = 0; STREAM = 1; }  enum Lane { INTERACTIVE = 0; BULK = 1; }
enum Select { ROUND_ROBIN = 0; LAST_ACTIVE = 1; }
message HelloAck { uint32 protocol_version = 1; string identity = 2; uint64 connection_id = 3;
  uint32 max_frame = 4; uint32 max_in_flight = 5; uint32 stream_window = 6;
  uint32 connection_window = 7; bytes credential = 8; string server_id = 9; }
message Call { uint64 id = 1; string target = 2; string method = 3; bytes payload = 4;
  string from = 5; string trace_id = 6; uint32 timeout_ms = 7; string content_type = 8;
  bytes credential = 9; uint32 window = 10; }
message Reply { uint64 id = 1; oneof result { bytes payload = 2; Error error = 3; }
  string content_type = 4; }
message Chunk { uint64 id = 1; bytes payload = 2; string content_encoding = 3; }
message End { uint64 id = 1; bytes payload = 2; }
message Credit { uint64 id = 1; uint32 bytes = 2; }
message Cancel { uint64 id = 1; string reason = 2; }
message Event { string subject = 1; bytes payload = 2; string content_type = 3; string from = 4;
  string scope = 5; }
message Subscribe { uint64 id = 1; string pattern = 2; }
message Unsubscribe { uint64 id = 1; }
message CredentialUpdate { bytes credential = 1; }
message Activate {}
message ConnectionError { string code = 1; string message = 2; }
message Error { string code = 1; string message = 2; }
```
- `protocol_version` 1 (B14). Hello first; anything else first, a bad version, a bad access
  token or credential: `ConnectionError` then close (the codes in `component-bus` spec).
- Unknown envelope variants and fields are ignored by both sides (proto3 and the repo's rule).
- `Call.id` is chosen by the caller per connection; the broker remaps ids per hop, so a reply
  is accepted only from the connection the call went to (a late or forged reply is dropped).
- A stream is a `Call` to a method whose `MethodSpec.kind` is `STREAM`: the provider answers
  `Chunk*` then `End` (optional final payload) or a `Reply` with an error; `Reply` with a
  payload is not valid for a stream (the broker turns it into `bad_reply`).
- An envelope that does not decode on a WebSocket gets `ConnectionError{bad_frame}` and the
  connection stays (message boundaries are intact, as `tilr.v1`'s "a malformed packet gets an
  error"); on a stream socket a bad length or an undecodable frame closes the connection
  (boundaries lost).

#### B5. Calls
Routing: `target` names a participant (identity) or is empty; empty routes by method over the
live, non-draining providers by the method's `Select` (B11 for `LAST_ACTIVE`; round-robin in
connection order otherwise). Methods are `<namespace>.<name>` (`[a-z0-9_-]` segments, at
least two). Namespaces `bus`, `pane`, `server`, `fs`, `exec`, `clipboard` are provided by
`tilrd` only; a Hello providing one is refused (`ConnectionError{reserved}`).

Deadlines and cancellation (gRPC deadlines, propagated): the broker uses `min(timeout_ms,
call_timeout_ms)` (default 10 s; 0 = the default; streams have no default deadline) and
forwards the time left; on timeout or the caller's connection ending it sends `Cancel` to the
callee and answers `timeout` / drops the call. A caller cancels its own call or stream with
`Cancel{id}` (gRPC cancel, HTTP/2 `RST_STREAM`); the broker forwards it.

Ordering: calls from one connection to one provider connection are delivered in the order sent
(a per-pair FIFO; the core handles them in that order). Nothing is promised across providers.
This keeps `tilr.v1`'s "input in the order it arrived" (T3).

Errors: closed set for the broker, open for handlers: `denied`, `not_found`, `unavailable`,
`timeout`, `busy`, `too_large`, `bad_request`, `handler_error`, `bad_reply`, `not_supported`,
`cancelled`, `spawn_failed`, `start_failed`, `start_timeout`, `bad_config`.

Limits: at most `max_in_flight` (256) calls and streams pending per caller; more → `busy` at
once. A callee whose outbound queue is full → `busy`.

#### B6. Events
Owner's decision: subjects, exact and wildcard subscriptions, delivery only to subscribers
connected now and allowed to subscribe, at-most-once, no history, the broker stores nothing;
state is caught up with a call. This is NATS core (subjects, `*` and `>`, no persistence) and
D-Bus signals (match rules, delivered to the connected).

- Subject: `<segment>(.<segment>)+`, segments `[a-z0-9_-]+`. Pattern: segments or `*` (one
  segment) or a trailing `>` (one or more), NATS syntax. `pane.*` matches `pane.bell`;
  `pane.>` also `pane.x.y`.
- `Subscribe{id, pattern}` is authorized once (`subscribe` right, R3) and answered with a
  `Reply{id}` (empty) or a `Reply` error; `Unsubscribe{id}` ends it; the connection's end ends
  all. Events carry the broker-set `from`.
- Emitting needs the `emit` right; the namespaces of B5 are emitted by `tilrd` only.
- Delivery: in emit order per (emitter, subscriber); a subscriber matching several of its
  patterns gets the event once. Per-event authorization: an event with a `scope` (set only by
  the core: `pane:<id>`) reaches a subscriber only if its token may access that resource (R6);
  decisions cached (R7).
- Slow subscriber: events go on the interactive lane; for a subscriber whose lane lags (B8)
  events are dropped, not queued, and counted (`tilrd_bus_events_dropped_total{subject}`);
  never a reason to drop the connection. NATS drops for slow consumers the same way; a full
  lane of other traffic still closes with 4001.
- Nothing after reconnect: a subscriber catches up by a call (`pane.list` after
  `pane.created`). Rejected: replay windows or durable subscriptions (JetStream, Kafka): the
  broker would store, and the owner decided it stores nothing.

#### B7. Streams and backpressure
Owner's decision: one-way streams, call → `Chunk*` → `End | Error`, cancelled by the caller,
inheriting the call's rights, with backpressure. No two-way channels; uploads are sequences of
calls (T5, as S3 multipart and tus do).

Compared:
- HTTP/2 flow control (RFC 9113 §5.2): byte credits per stream and per connection,
  `WINDOW_UPDATE`, initial 64 KiB, the receiver grants. Proven, bounds both a single stream
  and the sum on a connection.
- gRPC: uses HTTP/2's windows (with BDP probing to grow them); a slow reader stalls the
  writer's `Send`. Same model, inherited.
- Reactive Streams / RSocket `request(n)`: credits in messages. Simple, but chunks here range
  from 100 B (a one-row diff) to megabytes (a frame with images, a file chunk): message counts
  do not bound memory.
- TCP backpressure alone: through a broker, one slow stream would stall the whole connection
  and every other stream and event on it (head-of-line blocking; why HTTP/2 added per-stream
  windows).

Chosen: **HTTP/2-style byte credits, per stream and per connection, end to end through the
broker.** The caller gives the stream's initial window in `Call.window` (0 = `HelloAck.
stream_window`, 1 MiB; at most 16 MiB) and returns credit with `Credit{id, bytes}` as it
consumes chunks; the provider sends at most the credit it holds (sum of payload bytes). The
broker relays credits, enforces them (a provider that overruns gets the stream ended with
`bad_reply` and `Cancel`), and keeps a per-connection window for the consumer
(`HelloAck.connection_window`, 1 MiB for the bulk lane; the sum of uncredited bulk bytes to a
connection waits beyond it), which is `tilr.v1`'s `BULK_WINDOW` measured by the reader's own
credit instead of marked pings. A consumer that does not credit stalls its streams, never the
others.

Lanes: each method's `MethodSpec.lane` puts its chunks (and replies) on the consumer
connection's interactive or bulk lane (`outbox.rs` kept): interactive first, bulk only when the
interactive lane is empty; bulk bounded at 4 MiB queued per connection (`BULK_BYTES`).

Latest-wins streams: a provider may treat exhausted credit as "lagging" instead of waiting.
`pane.output` does (T4): it keeps today's lag rule (the consumer connection's interactive lane
held over 250 ms, or no credit) → skip diffs → full frame once caught up. A provider in Deno
or Python waits for credit (the SDK's `stream.write` awaits it).

Cancel: `Cancel{id}` from the caller ends the stream (the provider stops; chunks after it are
dropped); a provider ending early sends `End` or an error `Reply`. A stream ends with
`unavailable` when its provider's connection ends or tilrd reloads (B13).

#### B8. Bounded queues, no global lock
Per connection: interactive lane 1024 envelopes (full → close 4001 `too slow`, as today),
bulk lane 4 MiB; per caller 256 pending calls (`busy`). The routing table is read-mostly;
pending calls are kept per callee connection; ids come from an atomic counter; per-call timers
sit in one timer wheel, removed with the reply. A slow participant delays only its own calls.
These fix the prototype's unbounded queues, single mutex and never-cancelled timers (§10).

#### B9. Payloads
The broker never reads payloads except the core's own requests. Core methods (`bus.*`,
`pane.*`, `server.*`, `fs.*`, `exec.*`, `clipboard.*`) and the methods the core calls
(`host.open`, `ssh_agent.*`) use protobuf messages from `crates/tilr-bus/proto/tilr/<ns>/v1/`;
`content_type` `application/x-protobuf` (the default); a request with `application/json`
(proto3 canonical JSON mapping) is answered in JSON, so `tilr bus call pane.list` and
scripts need no codec. Events and stream chunks of the core are protobuf only (one encoding
per event, the broker does not convert); the SDKs ship the generated code. Rejected: JSON
everywhere (the prototype's control bodies; frames would grow several times and be re-encoded
per subscriber); CBOR/MessagePack (a second schema language).

`Chunk.content_encoding` `zstd`: the producer compressed the payload; the WebSocket framing
then sends that envelope plain (flag 0), so a frame is compressed once, in the pane process,
as today, and forwarded unchanged. A frame whose encoding would pass `max_frame` (images) is
split by the pane process: its images go first in chunks of kind `images` (B4 payload
`PaneOutput{oneof frame | images}`), then the frame without them; a single image larger than
`max_frame` is left out with a log line, as the 64 MiB cap would have dropped the frame.

#### B10. Config `bus.toml`, instances and supervision
Kept from the first round (`component-bus` D6-D9), with the owner's `rights` key:

```toml
call_timeout_ms = 10000      # default 10000
drain_ms = 3000              # default 3000
[modules.deno]
command = ["deno", "run", "-A", "components/deno/main.ts"]   # required, non-empty
cwd = "~/tilr/components"    # absolute or ~; default the config dir
[modules.deno.env]
LOG = "info"
[instances.deno-1]
module = "deno"              # required
connect = "fd"               # "fd" (default) | "socket"
autostart = true             # default true
[instances.deno-1.rights]
call = ["math.*", "pane.read"]
provide = ["ui.*"]
emit = ["deno.*"]
subscribe = ["pane.*"]
control = ["list", "reload:deno-1"]
pane = ["profile:agent", "access:own"]
[instances.deno-1.env]
WORKERS = "2"
```

- Its own file, strict TOML, read at start and on `bus.reload_config` (diff applied), next
  to `server.toml` which is read only at start: systemd keeps units apart from `system.conf`
  the same way. A bad file at start: logged, no instances, panes served; on reload:
  `bad_config`, nothing changed.
- Spawn: own session (`setsid`), stdin `/dev/null`, stdout/stderr into the log as
  `bus <name>: <line>`, every other descriptor close-on-exec; env = tilrd's minus the pane
  variables + module `env` + instance `env` + `TILR_INSTANCE`, `TILR_CONFIG_DIR`; `fd`:
  socketpair end as fd 3, `TILR_BUS_FD=3` (systemd socket activation's `LISTEN_FDS` model);
  `socket`: `<runtime dir>/bus/<name>.sock` (0600 in 0700), `TILR_BUS_SOCK`,
  `TILR_BUS_TOKEN` (128-bit hex, per process start, consumed once in constant time),
  `sun_path` checked at load (104 macOS / 108 Linux).
- Supervision (systemd `Restart=always` with backoff): 200 ms doubling to 5 s, reset after
  10 s up; Hello within 30 s of spawn; stop = SIGTERM to the group, 3 s, SIGKILL; states
  `down|starting|up|draining|stopped`. **A lost connection restarts the instance** (owner):
  its identity is the fd; it cannot reconnect.
- Control (`bus.instances`, `bus.methods`, `bus.start`, `bus.stop`, `bus.reload {instance |
  module | all}`, `bus.reload_config`), serialized in one queue; a caller that reloads itself
  gets its reply first.
- Reserved instance names: `owner`, `tilrd`, names starting with `bus`.

#### B11. Two providers of one method; `Activate`
Round-robin is right for workers (`math.add` on two Deno instances; NATS queue groups,
D-Bus has no load-balancing). It is wrong for UI: two Tilr Web windows both providing
`ui.notify` must not alternate. A provider declares `Select::LAST_ACTIVE` for such a method
and sends `Activate{}` when it becomes the user's active one (window focus); calls without a
target go to the provider that activated last among the live ones, else to the last connected.
This is how the repo already picks a client for the SSH agent (the one that subscribed last)
and how X11/macOS route input to the focused window. A caller may always name a `target`.
No provider live → `unavailable` (`no provider of ui.notify is connected`). Rejected:
broadcast to all windows (that is an event, not a call: one answer is expected); first come
(the oldest window, often hidden).

#### B12. Liveness and superseding
WebSocket transports keep `client-liveness` as it is: the server's pings (also answered by
browsers on their own), the silence timeout (4002), the 1024-envelope lane (4001), and
`Hello.client_id` superseding (4000) for the same principal. `bus.ping` (answered by the
broker) replaces `Ping`/`Pong` for clients that cannot send WebSocket pings. Stream sockets
(instances) need no pings: the peer is a child process (its exit is seen).

#### B13. tilrd's reload
Before the exec the broker stops routing new calls (`unavailable`, `tilrd is reloading`),
waits up to 1 s for calls in flight, answers the rest `unavailable` and sends `Cancel` to their
callees; open streams end with `unavailable`; subscriptions end with the connections that
carry them. **Instances and their connections survive** (owner): `bus-state 1` in
`server-<pid>.state` holds per instance its name, module, pid, state, connection fd (not
close-on-exec across the exec), listener fd for `socket`, provides, restart counters, the
consumed-token flag, its token's revocation ids and renewal time; their subscriptions are
handed over too (pattern list per connection), so an instance does not have to resubscribe.
WebSocket participants (owner, `tilr`, Tilr Web, scripts) reconnect as clients do today; panes
cross as they do. Frames an instance writes during the exec wait in the socket buffer.

#### B14. Protocol version and feature flags
README "Protocol" rule, unchanged: before the first public release every change keeps
version 1; afterward bump only for incompatibility with the previous public release, once per
release. `tilr.bus.v1` starts at 1; `tilr.v1`'s `PROTOCOL_VERSION` goes with the crate. The
switch itself is a new protocol, not a bump; old clients are told (B3). Feature flags: only
what a server can be run without, now in `server.info.features` (T2): `clipboard`, `files`,
`host-events`, `agent`, `exec`, `sandbox`. A feature off means its methods are not provided
(`not_found`, absent from `bus.methods`). No `bus` flag (it cannot be turned off; without
`bus.toml` there are no instances, `bus.*` answers), no `profiles` flag (owner). Client
features of `Hello.features` disappear: what a client can serve it provides (`host.open`,
`ssh_agent.request`), what it wants it subscribes to (`pane.copied`).

### Part R. Rights (Biscuit)

Kept from `bus-acl-biscuit` unless marked. Measured by the owner (Apple M2 Max, biscuit-auth
6.0.0, release): parse + verify + authorize 54-260 µs; authorize alone 5.6-23 µs; with a
parsed-token cache 7-29 µs; with a decision cache (TTL 1 s) 115-428 ns; some authorizations
hit the 1 ms default run limit under 12-thread load without the decision cache.

#### R1. Why Biscuit
Capability tokens with offline attenuation: Biscuit (Datalog, public-key signatures), as
macaroons (Google) and UCAN do; NATS uses signed JWTs with permissions (no attenuation without
the operator). Chosen by the owner. Rejected: ACL tables in tilrd (the prototype; no
delegation, the gateway needed every right); NATS-style JWT (no holder attenuation).

#### R2. Principals
`owner` (B2) holds every right and has no token. Every other participant presents a token:
instances get one minted at each start (`HelloAck.credential`), outside participants bring
one in `Hello.credential` (minted by the owner with `bus.token`). The root key is Ed25519 in
`<state dir>/bus-root.key` (0600, made on first start). Every token's authority block carries
`participant(<name>)`, its rights, `check if server(<server id>)` and `check if time($t), $t <=
<expiry>`.

#### R3. Rights (`rights` key, owner)
Config `[instances.<n>.rights]`, `tilr bus token --call … --provide … --emit … --subscribe …
--control … --pane …`:

| config | facts | allows |
|---|---|---|
| `call = ["m.x", "m.*", "*"]` | `call(m)`, `call_prefix(p)`, `call_any()` | calls and streams (a stream is a call); `*` excludes `bus.*` |
| `provide = [...]` | `provide…` | registering methods in Hello |
| `emit = [...]` | `emit…` | emitting events (reserved namespaces never) |
| `subscribe = [...]` | `subscribe_prefix(p)`, `subscribe_any()` | subscribing to patterns inside the prefix |
| `control = ["list", "reload:t-*"]` | `control(a, i)`, `control_any(a)` | `bus.*` control |
| `pane = ["profile:agent", "profile:*", "unrestricted", "override", "access:own", "access:any"]` | `pane_profile(n)`, `pane_profile_any()`, `pane_unrestricted()`, `pane_override()`, `pane_access(own\|any)` | T7 |

The first round's `calls` key is a config error naming `rights` (pre-release, no users).
`mint()` / `revoke()` rights are not given to instances by default.

#### R4. `Call.credential` is a bearer (owner)
A non-empty `Call.credential` authorizes that call alone; the connection's own token is not
consulted; `from` stays the connection's identity; audit names both. This is how OAuth bearer
tokens and macaroons are presented by an intermediary. Rejected: intersecting both (a
forwarder would need every right of every caller). Nested calls do not inherit an incoming
credential; an SDK forwards it only when asked.

#### R5. Expiry, renewal, revocation, rotation (owner)
- Every token expires after at most 24 h (short-lived credentials, as SPIFFE SVIDs).
  Instances: renewed by the broker after 12 h with `CredentialUpdate` on their connection.
  Outside participants: `bus.renew` returns a fresh token with the same authority block for a
  valid, unrevoked token **without appended blocks** (an attenuated token cannot be renewed:
  Biscuit cannot widen an expiry check by appending, and re-minting the authority block would
  drop the narrowing; the holder renews its root token and attenuates again); the participant
  presents it with `CredentialUpdate` on its connection; the old token is revoked.
- A connection whose token expires, is revoked or rotated away without an update is closed
  with close code **4003** `credential` (new; part of version 1, a contract like 4000-4002).
- Revocation: `bus.revoke {token | revocation_id}` and every instance's token at its process
  end; the set is kept until expiry, in memory and `<state dir>/bus-revoked` (0600), checked on
  every call (also on decision-cache hits).
- Root key rotation (`bus.rotate_key`): a new key; **every token dies at once** (owner; no
  overlap, no `root_key_id`); instances get new tokens by `CredentialUpdate` (no restart:
  their identity is the fd); outside participants' connections close with 4003. NATS operator
  key rotation also re-issues everything; an overlap window (`kid`, JWKS) is what this
  rejects.

#### R6. Authorization facts
Per decision the broker supplies (never the token): `caller(i)`, `method(m)`,
`namespace(n)`, `target(t)`, `server(id)`, `time(now)`, `origin(o)` (the WebSocket `Origin`,
"" without), and per kind: `action(a)`, `subject(s)` (control); `provide(m)`; `emit(s)`;
`subscribe(p)`; `scope(r)` with, for `pane:<id>`, `pane_creator(c)` (per-event and per-pane
checks). Policies as `bus-acl-biscuit` D1 (`starts_with` for prefixes, `deny if true` last);
appended blocks can only add checks (authority facts are scoped).

#### R7. Verification cost (owner: 1 ms + cache)
(1) SHA-256 of the token → parsed-token cache (LRU 1024; dies with expiry or rotation);
(2) revocation ids against the set (always); (3) decision cache (token hash, kind, caller,
method/subject/pattern, target, scope, origin) → allow/deny, TTL 1 s, cleared on revocation,
rotation and config reload; (4) on a miss a fresh authorizer with run limits 1 ms, 1000 facts,
100 iterations; token size ≤ 16 KiB, ≤ 8 blocks. Any error or limit = `denied` (fail closed).
A token may be used up to 1 s past expiry (decision TTL), never after revocation.

#### R8. Audit
`bus audit:` lines (target `bus::audit`): every denial (caller, first revocation id 16 hex,
kind, method/subject, target, reason), every control, mint, renew, revoke and rotation call,
allowed or not; allowed calls only at debug. Token bytes never logged.

#### R9. No "sandbox only" token kind (owner)
A right `pane = ["profile:agent"]` without `unrestricted` and `override` is exactly "may create
only sandboxed panes of profile agent". `read-only-viewer`'s token kinds are not needed for it.

### Part W. Tilr Web on the bus

#### W1. A participant like any other (owner)
The Tilr app's window (Tilr Web) and each JS plugin in it connect over WebSocket with
subprotocol `tilr.bus.v1` and speak the same envelopes (nats.ws gives browsers NATS core the
same way); a plugin can call (`pane.create`, a Python instance's `nlp.summarize`), subscribe
(`pane.*`), stream (`pane.output`) and **provide** methods (`ui.notify`, `ui.pick_file`):
Python calls `ui.notify`, the plugin's handler runs in the page and answers. No window
providing it → `unavailable`.

#### W2. Origin
The existing check stays and applies to the bus handshake: a request with an `Origin` header
is accepted only for an allowed origin (`--allow-origin`, `allow_origins`, built-in
`tilr://app`, `tilr://tilr`, `http://127.0.0.1:7891`, `http://localhost:7891`), else HTTP 403;
without `Origin` (not a browser) it passes. On the bus, the origin is also an authorization
fact (R6): tokens minted for a page carry `check if origin("tilr://app")`, so a token copied
into another allowed page or a non-browser program is refused (`denied: origin`). Browsers set
`Origin` and pages cannot forge it, the reason the WebSocket RFC (6455 §10.2) makes it the
server's check; binding the token to it is the same idea as cookie `SameSite`.

#### W3. How a plugin gets its token
The app's host connection is the owner (TCP access token or the Unix socket, as today). For
each plugin it mints a token with `bus.token {name: "web:<plugin id>", rights from the plugin's
manifest, origin, ttl ≤ 24 h}` and hands it to the plugin's page; the page connects with it,
renews it with `bus.renew` before expiry and attenuates it further if it delegates. The token
never leaves the app's process tree. A plugin's identity is `token:web:<plugin id>`, the same
across windows and reconnects, so panes it made stay `own` (T7). Rejected: plugins sharing the
host's owner connection (every plugin would hold every right); a token per page load without
renewal (a page open over 24 h would die).

#### W4. Two windows, one provider
Both windows provide `ui.notify` with `Select::LAST_ACTIVE` and send `Activate` on focus; a
call goes to the focused window, else the last connected (B11). Each window's connection has
its own `client_id`; closing one leaves the other providing; closing both → `unavailable`.
Events reach every subscribed window (at-most-once each).

### Part T. The terminal on the bus

#### T1. The core is a native participant
tilrd's core registers its methods as the participant `tilrd` (always up, never drained, not in
`bus.toml`), answers in process with the supervisor's existing paths (create, input with
acknowledgement, resize, close, frames, files, exec, clipboard), and emits the pane events.
`tilr.proto` is deleted; its types move:
- client-facing types (frames, input, pane info, files, exec, clipboard) to payload schemas
  `crates/tilr-bus/proto/tilr/{pane,server,fs,exec,clipboard,host,agent}/v1/*.proto`, field
  numbers kept where the message keeps its meaning (`Frame`, `Run`, `Style`, `PaneInfo`,
  `KeyInput`, ...), so `tilr-screen` and the pane process change only their import paths;
- internal schemas (pane link, reload state) to `crates/tilrd/proto/internal.proto`, package
  `tilrd.internal.v1`, not copied by any client; the dead legacy fields
  `PaneHandover.delivery_states`/`pending_delivery` and their messages are removed (reserved).

Rejected: keeping `tilr.proto` as a payload schema file (the owner removes it whole; one
schema per namespace makes each method's payload findable); a separate `tilr-panes` component
(direction of 2026-10-26; not now).

#### T2. Mapping of every `tilr.v1` message

`ClientMessage` (31):

| # | message | on the bus | notes |
|---|---|---|---|
| 1 | `Hello` | bus `Hello` (`protocol_version`, `client_name`, `client_id`, `access_token`, `credential`) | `features` → providing / subscribing (B14) |
| 2 | `CreatePane` | call `pane.create` | reply `PaneInfo`; `request_id` → call id; `profile` (P5) |
| 3 | `ClosePane` | call `pane.close` | reply `{exit_code}` once the program ended |
| 4 | `ListPanes` | call `pane.list` | only panes the caller may access (T7) |
| 5 | `Subscribe` | stream `pane.output` | first chunk a full frame (T4) |
| 6 | `Unsubscribe` | `Cancel` of that stream | generic, no message of its own |
| 7 | `Resize` | call `pane.resize` | carries `seq`, `input_seq` (T5) |
| 8 | `KeyInput` | call `pane.input {key}` | reply = acknowledgement (T5) |
| 9 | `MouseInput` | call `pane.input {mouse}` | |
| 10 | `Paste` | call `pane.input {paste}` | |
| 11 | `RawInput` | call `pane.input {raw}` | |
| 12 | `Scroll` | call `pane.input {scroll}` | credited at once, as today |
| 13 | `ClipboardPut` | calls `clipboard.put` (one per chunk, `transfer_id`, `last`) | reply to the last chunk = `ClipboardAck` |
| 14 | `Ping` | call `bus.ping` | answered by the broker |
| 15 | `FsList` | call `fs.list` | |
| 16 | `FsStat` | call `fs.stat` | |
| 17 | `FsRead` | stream `fs.read` (bulk) | chunks ≤ 256 KiB as today |
| 18 | `FsWrite` | calls `fs.write` (one per chunk) | file replaced after the `last` chunk, as today |
| 19 | `FsMkdir` | call `fs.mkdir` | |
| 20 | `FsRename` | call `fs.rename` | |
| 21 | `FsDelete` | call `fs.delete` | |
| 22 | `FsWatch` | call `fs.watch` → `{watch_id}`; events `fs.changed.<watch_id>` | watch ends with `fs.unwatch` or the caller's connection |
| 23 | `FsUnwatch` | call `fs.unwatch` | |
| 24 | `StreamData` (client → server) | the reply payload of `ssh_agent.request` | agent protocol is request/response (T6) |
| 25 | `StreamClose` (client → server) | an error reply of `ssh_agent.request` | the program's connection is closed |
| 26 | `ExecStart` | stream `exec.run` (bulk); with `server_command`: stream `server.command` | first chunk `started{exec_id, pid}` |
| 27 | `ExecInput` | call `exec.input` | reply `{taken}` replaces `ExecInputAck` |
| 28 | `ExecSignal` | call `exec.signal` | |
| 29 | `ExecCancel` | `Cancel` of the `exec.run` stream | generic |
| 30 | `FinalizeInput` | call `pane.input_finalize` | |
| 31 | `ClearPane` | call `pane.input {clear}` | |
| - | `ClientMessage.seq`, `input_seq` | fields of `pane.input` / `pane.resize` | |
| - | `input_identity`, `input_probe`, `input_forget` | internal pane link (`internal.proto`) | never on the bus |

`ServerMessage` (24):

| # | message | on the bus | notes |
|---|---|---|---|
| 1 | `HelloAck` | bus `HelloAck` (`protocol_version`, `identity`, `connection_id`, limits, `server_id`) + call `server.info` (`server_version`, `update_channel`, `features`, `clipboard_max_bytes`, `fs_max_bytes`) + call `pane.applied` (`applied_input`) | `seq_base` = `connection_id << 32` |
| 2 | `PaneCreated` | reply of `pane.create` | |
| 3 | `PaneList` | reply of `pane.list`; the push → events `pane.created`, `pane.changed`, `pane.exited` | a subscriber catches up with `pane.list` |
| 4 | `PaneExited` | event `pane.exited`; `End{exit_code}` of open `pane.output` streams | |
| 5 | `Frame` | chunk of `pane.output` (`PaneOutput.frame`) | T4 |
| 6 | `Error` | error `Reply` / stream error / `ConnectionError` | codes B5 |
| 7 | `ClipboardAck` | reply of the last `clipboard.put` | |
| 8 | `Pong` | reply of `bus.ping` | |
| 9 | `FsEntries` | reply of `fs.list` | |
| 10 | `FsInfo` | reply of `fs.stat` | |
| 11 | `FsData` | chunks of `fs.read` | a failed read ends with an error |
| 12 | `FsAck` | replies of `fs.write`, `fs.mkdir`, `fs.rename`, `fs.delete`, `fs.watch`, `fs.unwatch` | |
| 13 | `FsChanged` | event `fs.changed.<watch_id>` | `request_id` (always 0) dropped |
| 14 | `HostEvent` | call `host.open`, provided by the client, made by tilrd to each watcher of the pane that provides it | none → the program hears no client is attached |
| 15 | `StreamOpen` | the first `ssh_agent.request {conn_id}` | |
| 16 | `StreamData` (server → client) | the request payload of `ssh_agent.request` | |
| 17 | `StreamClose` (server → client) | call `ssh_agent.close {conn_id}` | |
| 18 | `ExecStarted` | first chunk of `exec.run` (`started`) or its error reply | |
| 19 | `ExecOutput` | chunks of `exec.run` (`output{stream, data}`) | |
| 20 | `ExecExit` | `End` of `exec.run` (`ExecExit` payload) | |
| 21 | `ExecInputAck` | reply of `exec.input` | |
| 22 | `ClipboardWrite` | event `pane.copied {pane_id, transfer_id, mime, total_size}` (scope `pane:<id>`) + stream `clipboard.get {transfer_id}` (bulk) | T6 |
| 23 | `PaneActivity` | events `pane.bell`, `pane.command_finished` | |
| 24 | `InputAck` | reply of `pane.input` / `pane.resize`; a missing pane → `not_found` reply (terminal) | |

Removed without a replacement of their own, because a generic bus mechanism now does their
job: `Unsubscribe` and `ExecCancel` (stream `Cancel`), `Pong` (a reply), `Error` (error
replies and `ConnectionError`), `StreamOpen` (implicit in the first request), `PaneList.
request_id`, `FsChanged.request_id` (call ids and subjects), `Hello.features` (providing and
subscribing). Removed outright: the legacy `PaneHandover` fields 16-17 with
`InputDeliveryState` and `PendingDelivery` ("no longer written or restored"). New, with no
`tilr.v1` counterpart: `pane.read`, `pane.profiles`, `server.reload`, `server.update`,
`clipboard.get`, `bus.*`.

`server.*`: `server.info` (version, channel, features, limits, id), `server.reload`
(as SIGUSR1; `control` right or owner), `server.update` (stream of `tilrd update`'s output,
then a reload as today), `server.command` (the former `ExecStart.server_command`: `token`,
`cloud login`, ...; owner on the Unix socket only, as today). Internal messages
(`PaneStatus`, `SupervisorState`, `PaneState`, `PaneHandover`, `PendingInput`,
`PaneInputAck`) move unchanged to `internal.proto`.

#### T3. Guarantees kept, and how the bus states them
| guarantee of `tilr.v1` | on the bus |
|---|---|
| frames first, data in quantity behind (lanes) | `MethodSpec.lane`: `pane.output`, replies, events interactive; `fs.read`, `exec.run`, `clipboard.get`, `server.update` bulk (B7) |
| bulk ≤ 4 MiB queued, ≤ 1 MiB unread in the network | bulk lane 4 MiB; per-connection window 1 MiB by the reader's `Credit` (replaces ping marks; pings stay for liveness) |
| a lagging client skips to the latest screen, still gets its echo | `pane.output` latest-wins (T4) |
| a client whose queue fills is dropped (4001) | same, per connection |
| input in order, acknowledged after application, replayed once across reconnect | per-pair FIFO (B5); reply after the pty write; `input_seq` + `client_id`; `pane.applied`, `pane.input_finalize` (T5) |
| latency seq echoed in frames | `pane.input.seq`, `seq_base` from `connection_id`, `Frame.last_input_seq` |
| reload with handover; screens, scrollback, programs, pending input carry over | unchanged for panes; instances too (B13); connections of WebSocket participants end and reconnect, streams end `unavailable` |
| `Hello` first, version check, nothing before `HelloAck` | bus Hello (B4); no event or chunk before `HelloAck` |
| unknown messages and fields ignored | unknown envelope variants and fields ignored; unknown methods `not_found` |
| a malformed packet gets an error, the connection stays | `ConnectionError{bad_frame}` on WebSocket (B4) |
| superseding by `client_id` (4000), silence (4002) | B12 |
| limits: 64 MiB decompression, `fs_max_bytes`, clipboard max, 1024 / 4 MiB queues, 4 MiB unacked exec stdin | 16 MiB `max_frame` (B9 splits frames), `server.info` limits, the same queues, `exec.input` kills at 4 MiB unacknowledged |
| feature flags | `server.info.features` (B14) |
| token on TCP, peer uid on the Unix socket, Origin, 10 s handshake | unchanged, plus Biscuit for non-owners (Part R) |

#### T4. `pane.output`
Request `{pane_id, cell_width_px?, cell_height_px?}`; stream of `PaneOutput` chunks; the first
is a full frame; then what the pane process produces, the same `Frame` messages, compressed
once by it (B9) and forwarded unchanged; `End{exit_code}` when the program exits. While the
consumer lags (its connection's interactive lane held a packet over 250 ms, or the stream's
credit is spent) diffs are skipped; when it has caught up (lane drained and credit back) the
pane is asked for a full frame, sent with that consumer's latest echoed `last_input_seq`. A
consumer counts as a viewer (`PaneInfo.viewed_by`) for the stream's life; `pane.read` callers
do not. Clients open it with a 4 MiB window (frames with images; `Call.window`); credit is returned per chunk decoded. This is
`tilr.v1`'s `Subscribe` behavior, measured the same way, so `screen-frames` and
`client-liveness` hold as written for subscribers.

#### T5. Input, acknowledgement, replay
`pane.input {pane_id, seq, input_seq, oneof key | mouse | paste | raw | scroll | clear}` and
`pane.resize {pane_id, cols, rows, cell_*_px, seq, input_seq}`; the reply comes when the input
was applied (bytes taken by the pty, or scroll/resize/clear applied), the former `InputAck`.
Clients do not wait for a reply before the next input (pipelining; ordered by B5's FIFO).
Numbered input (`input_seq` > 0) needs a `client_id` (else `bad_request: numbered input
requires a stable client_id`); deduplication per (`client_id`, pane) as today; `pane.applied
{}` answers the applied counters for the caller's `client_id` (the former
`HelloAck.applied_input`, with the 5 s probe bound); `pane.input_finalize {}` ends the
session. A missing pane: `not_found` `no such pane: <name as sent>`, terminal (the client
reclaims that input). Memory: a caller keeps ≤ 256 calls pending (B5) and the client library's
outbox ≤ 4 MiB / 4096 inputs as today.

#### T6. Copies, host events, the SSH agent
- `pane.copied` (scope `pane:<id>`): small event; the content is fetched with the bulk stream
  `clipboard.get {transfer_id}` (`not_found` once a newer copy replaced it). Delivery widens
  from "clients subscribed to the pane with `clipboard-write`" to "subscribers of
  `pane.copied` allowed to access the pane"; clients act on copies of panes they show.
- `host.open {pane_id, name, kind, path, args}`: tilrd calls it on each watcher of the pane
  (its `pane.output` consumers) that provides `host.open`; none → the program is told no client
  is attached, as today.
- SSH agent: a program connecting to the pane's `SSH_AUTH_SOCK` makes tilrd read one agent
  message (u32 length + body), call `ssh_agent.request {conn_id, pane_id, message}` on the
  watcher of the pane that provides it and subscribed last (today's rule), and write the reply
  back; `ssh_agent.close {conn_id}` when the program closes. The agent protocol is strictly
  request/response per connection, so no two-way channel is needed; `conn_id` lets the client
  keep one upstream agent connection per program connection (`session-bind@openssh.com`).
  Bulk-free (agent messages are small).

#### T7. Pane rights
The broker checks `call("pane.create")` etc.; the core authorizes again with the same token
and pane facts: `pane_profile_requested(n | "")`, `pane_overrides(bool)`, `pane(id)`,
`pane_creator(c)`, `caller(i)`:
- `pane.create` needs `pane_profile(n)` or `pane_profile_any()` (no profile:
  `pane_unrestricted()`), and with `command` or `cwd` in the request also `pane_override()`;
- `pane.list`, `.output`, `.read`, `.input`, `.resize`, `.close` and pane-scoped events: with
  `pane_access("own")` (the **default** when the token names none; owner) only panes whose
  `created_by` is the caller's identity; with `pane_access("any")` every pane;
- `PaneInfo.created_by`: `owner`, `instance:<name>` or `token:<name>` (the caller's identity,
  also when a per-call credential was used: the credential limits, the caller owns); handed
  over in `PaneState.info`.
Docker's per-container ownership and Kubernetes owner references model the same "own" rule.

#### T8. Panes outlive their creator (owner)
A pane made by an instance or a plugin keeps running when its creator stops, crashes or
reloads; the creator's next connection (same identity) accesses it again. `close_with_creator`
is a possible later option, not now.

#### T9. `pane.read`
`{pane_id, since_seq?, wait_ms? (≤ 25000), history_lines? (≤ 10000)}` → `{seq, cols, rows,
lines, cursor, title, cwd, alive, exit_code?, history}`: the screen model's text (from the
pane process, rows trimmed, wide characters once); with `since_seq` it waits for a change
after it or the exit, at most `min(wait_ms, 25 s, time the call has left − 100 ms)`. This is
tmux `capture-pane` + `wait-for`: automation reads what the user sees, not escape sequences.
For a live feed, `pane.output` is the stream.

#### T10. Clients in this repo
`tilr` (attach, new, ls, ps, kill, kill-server, update, token, cloud, profiles, bus),
`tilrd ps`, `tilr-driver` and the test harness (`crates/tilrd/tests/common/mod.rs`) use the
Rust client of `crates/tilr-bus`; the itests drive `tilr-driver` as before. Test names stay
(they state use cases); the messages inside change.

### Part P. Launch profiles (from `pane-profiles`)

#### P1. Where profiles live (owner: `profiles_dir`)
`server.toml` key `profiles_dir` (absolute or `~`); default `profiles/` in the directory of
the `server.toml` in use (`$TILR_CONFIG_DIR`, `$XDG_CONFIG_HOME/tilr`, `~/.config/tilr`, or the
directory of `--config` / `TILR_CONFIG`). A missing directory = no profiles. A key rather than a
variable, so services and tests keep their whole configuration in one file, as systemd drop-ins
sit next to their unit. **Changes** the first round's "no own key" (`pane-profiles` D1).

#### P2. Names, never paths; StrictModes (owner)
Names `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`, refused before any file access; the server joins
`profiles_dir/<name>.toml`. A file may be a symlink to a regular file. The file (after
symlinks) and the directory must be owned by the server's user and not writable by group or
others, else the profile does not load: `profile <name>: <file> is writable by others` (or `is
owned by uid N`). This is ssh's `StrictModes` for `authorized_keys`: a sandbox profile anyone
can edit turns the sandbox off.

#### P3. Format
Strict TOML (`deny_unknown_fields`): `description` (optional, owner), `command` (argv; absent or
empty = login shell), `cwd`, `[env]`, optional `[sandbox]` with `read_only`, `read_write`,
`network` (`none` default | `client` | `server`), `executables`, `env`. Paths absolute or `~`;
relative is an error (deviation from the prototype: profiles live in the config directory, not
in a project); `.`/`..` folded lexically; `[env]` names non-empty, without `=`/NUL, not
`TILR_*`; a leading `~/` in values expanded. No `path` key (`PATH` through `[env]` or the
sandbox default).

#### P4. Read on every create, snapshotted per pane
Read and validated on every `pane.create` naming it and every `pane.profiles`, never cached
(the prototype's model); the resolved snapshot (argv, cwd, env, sandbox) is the pane's for life
and across reloads (`PaneState.profile`, `supervisor-state 3`).

#### P5. The request overrides the command, the sandbox stays (owner)
A non-empty `pane.create.command` replaces the profile's; a `cwd` naming an existing directory
wins, else the profile's, else home; nothing in the request changes `[env]` or `[sandbox]`. The
sandbox applies to whatever runs: `docker run image cmd` runs `cmd` with the image's
confinement. Whether a restricted caller may override at all is a right (`pane_override`, T7),
not a profile key.

#### P6. Listing
`pane.profiles {}` → `{profiles: [{name, description, sandboxed, error}]}` sorted by name; files
whose stem is not a valid name or that do not end in `.toml` skipped; at most 1000 listed;
`description`/`error` cut at 1 KiB. Needs `call("pane.profiles")` (owner's configuration).

#### P7. Errors
`pane.create` refusals are `bad_request` with today's texts: `create pane: bad profile name:
<n>`, `create pane: no such profile: <n>`, `create pane: profile <n>: <file>: <reason>`,
`create pane: profile <n> asks for a sandbox, which this server cannot enforce` (no `sandbox`
feature), `create pane: sandbox: <reason>`.

#### P8. No feature flag (owner)
Profiles cannot be turned off; the first round's after-the-fact check in `tilr new` (an old
server ignoring `profile`) is not needed: a server without profiles does not speak the bus.

### Part S. Sandboxed panes (from `sandboxed-pane`, content kept)

History of the macOS choice: 2026-10-18 App Sandbox with a signed pane binary and a quarantine
experiment first; the experiment is answered (files made inside an App Sandbox get
`com.apple.quarantine` and cannot run inside); 2026-11-17 Seatbelt after the probe (S10). Linux
Landlock stands; seccomp is added (S5, confirmed by the owner).

#### S1. The request comes from the profile
No sandbox fields in `pane.create`; a caller asks for a sandbox only by naming a profile.

#### S2. Goals of the mechanism
A sandboxed pane is a normal terminal (raw mode, TUIs such as vim, less, Claude Code, job
control, `stty`); scripts and binaries made inside run inside; nothing inside reaches the
server, its sockets, the token or the user's agent; fail closed; survives a reload; cleans up.

#### S5. Linux: Landlock plus seccomp (owner: confirmed)
Applied in the shell child's `pre_exec` after `setsid`/`TIOCSCTTY`, before `execve`:
- `prctl(PR_SET_NO_NEW_PRIVS)`, then a Landlock ruleset handling every filesystem right of the
  kernel's ABI (no best-effort downgrade); **minimum ABI 4 (kernel 6.7)** for the `sandbox`
  feature (owner):
  - `read_only`: read file, read dir, execute; `read_write`: all filesystem rights;
  - `executables` (resolved through the pane's `PATH` at creation) and the program actually
    run: read and execute; their interpreter and libraries come from the system directories;
  - always: read+execute on `/usr`, `/bin`, `/sbin`, `/lib`, `/lib32`, `/lib64`; read on
    `/etc`, `/proc`, `/sys`; read+write on `/dev/null`, `/dev/zero`, `/dev/full`,
    `/dev/random`, `/dev/urandom`, `/dev/tty`, `/dev/ptmx`, `/dev/pts` (with `IOCTL_DEV` where
    ABI ≥ 5); the pane's private `TMPDIR` (S8);
  - network (ABI ≥ 4): `client` handles `BIND_TCP` with no port allowed and leaves
    `CONNECT_TCP` free; `none` handles both with nothing allowed; `server` handles neither.
    **`client` does not restrict UDP** (Landlock has no UDP rules): accepted by the owner and
    written in README as a limit;
  - **ABI ≥ 6: `LANDLOCK_SCOPE_SIGNAL` and `LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET`; on ABI 4-5
    the pane starts without signal scoping and the server logs a warning for each such pane**
    (`sandbox: kernel lacks signal scoping (Landlock ABI N < 6); pane <id> can signal the
    user's other processes`).
- seccomp-bpf after Landlock: **`socket(AF_UNIX, …)` → `EACCES` always** (owner; Landlock does
  not govern `connect` to named Unix sockets, and the server's, host-events and agent sockets
  accept any process of the user); `socketpair` allowed; with `network = "none"` also
  `socket(AF_INET|AF_INET6, SOCK_DGRAM|SOCK_RAW)` → `EACCES`.
- Job control and raw mode are untouched (fds 0-2 opened before the restriction; no ioctl is
  filtered).
Rejected: bubblewrap / user namespaces (disabled or AppArmor-restricted on Ubuntu ≥ 23.10; a
setuid helper is out of scope); seccomp only (no path rules). Chromium, Firefox and Flatpak
combine a filesystem policy with seccomp the same way.

#### S6. What a sandboxed pane does not get
- Environment from scratch: `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `COLORTERM`,
  `TILR_PANE`, `TMPDIR`, `PATH`, the `[sandbox] env` names with the server's values, then
  `[env]`. `PATH` default: the `executables`' directories, then `/usr/bin:/bin:/usr/sbin:/sbin`.
  Never: `TILR_SHIMS`, `TILR_SOCK`, the server socket variable, the agent's `SSH_AUTH_SOCK`,
  any `TILR_BUS_*`.
- No login wrappers, no shims; the login shell starts as `-<shell>`.
- **Grants covering the state directory (token, root key), the runtime directory (sockets) or
  the configuration directory (the profile itself, `bus.toml`) are refused** (owner; for the
  configuration directory also `read_only`, since `bus.toml` and profiles reveal rights):
  `create pane: sandbox: <path> would expose <what>`. Landlock grants by allow list only, so a
  grant of `~` cannot carve them out; refusing is the fail-closed answer on both platforms
  (`read_only = ["~"]` is refused when the state directory is under home, its default).
- A sandboxed pane is not a bus participant: no `AF_UNIX`, no bus variables.

#### S7. Fail closed
No `sandbox` feature → the profile is refused (P7). Applying fails (Landlock or seccomp error,
`sandbox-exec` missing or refusing the profile) → the child exits before `execve`, the pane
process reports `create pane: sandbox: <reason>`, nothing runs unrestricted. A grant path that
does not exist is skipped with a log line. The server never falls back to an unrestricted pane,
also not after a reload.

#### S8. Temporary directory and cleanup
`<runtime dir>/sandbox/<pane id>/` (0700) as `TMPDIR`, removed when the pane ends; at start the
server removes entries whose pane id is not a running pane. On macOS it also holds the
generated profile; nothing else.

#### S9. Feature and pane info
`PaneInfo.sandboxed` (bool). `sandbox` in `server.info.features` when the start-up check
passes (Linux: Landlock ABI ≥ 4 and seccomp; macOS: S10's check).

#### S10. macOS: Seatbelt with a generated deny-default profile (owner, 2026-11-17)

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
version in CI (**macOS 14, 15 and 26 in the merge queue, release branches and nightly**, owner;
pull requests keep the Linux-only quick tier), and an unavailable `sandbox-exec` or a profile
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
  `TMPDIR` (S8); `process-exec` on the system program directories (`/usr/bin`, `/bin`,
  `/usr/sbin`, `/sbin`, `/usr/libexec`) and on the trampoline.
- `read_only` → `(allow file-read* (subpath (param "RO_n")))` (a file: `literal`), plus
  `process-exec` under it (the same "read and execute" as Linux S5).
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
  denied by `(deny default)` — the macOS counterpart of Linux S5's `AF_UNIX` rule. As defense in
  depth the profile ends with `(deny file-read* file-write* network-outbound (subpath (param
  "DENY_n")))` for the state, runtime and configuration directories (later rules win in SBPL), in
  addition to S6's refusal of grants that cover them.
- `env`: handled outside the profile, by the environment built in S6 and passed to
  `sandbox-exec`'s child (Seatbelt does not filter variables).

**Start-up check** (`sandbox` feature on macOS): `/usr/bin/sandbox-exec` is executable and a
fixed probe profile applies to `/usr/bin/true` within 2 s.

**Cleanup.** Only the per-pane directory of S8 (profile and `TMPDIR`); no bundles or containers.

(S3/S4 numbers are the first round's Linux draft and "request from the profile"; S3 is folded
into S5, S4 into S1.)

### Part K. SDKs (from `bus-runtime-sdks`)

#### K1. Packages (owner)
`sdk/deno/` (`mod.ts`, `deno.json`, Deno 2.x) — also the browser library: the same module,
transport chosen by the environment (fd/instance socket via `node:net` under Deno, WebSocket in
a page or under Deno with a URL); `sdk/python/` (`tilr_bus`, Python 3.12, `uv`; transports fd,
instance socket, WebSocket via `websockets`). Outside `crates/` (not Cargo crates). Not
published to JSR, PyPI or npm until the release that makes the protocol public (owner); used by
path or git; the Tilr app bundles `sdk/deno` for its plugins.

#### K2. Surface (owner: call, provide, emit/on, stream)
| | Deno / browser | Python |
|---|---|---|
| connect | `await Bus.connect({url?, credential?})` (env: `TILR_BUS_FD`, `TILR_BUS_SOCK`+`TILR_BUS_TOKEN`, `TILR_BUS_URL`+`TILR_BUS_CREDENTIAL`) | `await Bus.connect(...)` |
| call | `bus.call(m, payload, {target, timeoutMs, contentType, credential})` | `await bus.call(m, payload, ...)` |
| provide | `bus.provide(m, handler, {lane, select})` before connect; `bus.activate()` | `@bus.provide(m)` |
| stream (caller) | `for await (const chunk of bus.stream(m, payload, {window}))`; `break` cancels | `async for chunk in bus.stream(...)` |
| stream (provider) | `bus.provide(m, async (req, ctx, out) => { await out.write(bytes) })` (awaits credit) | `async def h(req, ctx, out): await out.write(b)` |
| emit / on | `bus.emit(subject, payload)`; `bus.on(pattern, handler)` → unsubscribe | `await bus.emit(...)`; `bus.on(pattern)` |
| context | `ctx.from`, `ctx.method`, `ctx.traceId`, `ctx.deadline`, `ctx.signal`, `ctx.credential`, `ctx.call` | same, `ctx.cancelled` |
| identity, token | `bus.identity`, `bus.credential` (latest `CredentialUpdate`; `bus.renew()`) | same |
| core types | generated `tilr.pane.v1` etc. and typed helpers (`bus.pane.create(...)`) | same |

#### K3. Errors, limits, cancellation
`BusError(code, message)` with B5's codes; a handler's other exception → `handler_error`
(type and message, no traceback to the caller); an unregistered method → `not_supported`.
Writes wait for the socket (Deno `drain`, Python `await writer.drain()`, browser
`bufferedAmount`); at most `max_in_flight` own calls pending (later ones wait); over
`max_frame` → `too_large` locally; `Cancel` aborts the handler (`AbortSignal`, task
cancellation) without a reply; credit returned as chunks are consumed.

#### K4. One schema
`scripts/gen-bus-sdk.sh [--check]` generates from `crates/tilr-bus/proto/` (bus and payload
schemas): Python `*_pb2.py` with the pinned `protoc`; Deno a module holding the schema for
`protobufjs` (pinned, `deno.lock`); checked in; `--check` fails CI on drift.

#### K5. Tests and CI
Each SDK has a test runtime and tests against a real `tilrd` (temporary config, state and
runtime directories, `--no-listen` plus a test port for WebSocket): both instance transports,
WebSocket with a credential, providing, events, streams with slow consumers, cross-language
calls (Python ↔ Deno, browser-style WebSocket client ↔ Python). CI job `sdk` (Linux on pull
requests; Linux and macOS in the merge queue and nightly), `scripts/sdk-check.sh` locally.

### Part O. Organization

#### O1. One change (owner)
The whole phase is this change: one task, one branch, one PR, one switch. Capabilities stay
separate spec files. The seven first-round directories are removed (kept outside the repo as a
backup of this session); `bus-gateway` is deferred in `openspec/ROADMAP.md` with its reason and
no directory.

#### O2. Order inside the branch
tasks.md orders the work so the tree builds at every step: the bus crate and broker beside
`tilr.v1`, then the core methods, then the clients and tests switched together with the
removal of `tilr.proto`, then profiles, sandbox, Tilr Web, SDKs, docs. The PR is squash-merged;
`main` sees one switch.

## Reload and reconnect

- tilrd reload: B13 (instances, connections, tokens, subscriptions of instances handed over;
  calls answered `unavailable`; streams end); panes as today plus `PaneState.profile` and
  `created_by` (`supervisor-state 3`), `bus-state 1`; sandboxes stay in the kernel on the
  program's tree (S5, S10); caches start empty.
- WebSocket participants reconnect: a new connection, a new `connection_id` (`seq_base`), the
  same `client_id`; they subscribe again, reopen `pane.output` (full frame first), call
  `pane.applied` and replay unacknowledged input (T5).
- An instance's own restart: new process, new token (old revoked), resubscribes; its panes
  stay (T8).
- `tilrd state-formats`: `supervisor-state 3`, `pane-state 3`, `bus-state 1`, `unix-socket 1`,
  `reload-check 1`.

## Backpressure

B7 and B8: credits per stream and per connection, two lanes per connection, 1024 interactive
envelopes and 4 MiB bulk per connection, 256 pending calls per caller, `busy` instead of
growth, events dropped for a lagging subscriber, `pane.output` latest-wins, uploads paced by
their replies, `exec.input` ≤ 4 MiB unacknowledged. Authorization cost bounded by R7.

## Risks / Trade-offs

- **One big switch**: a long branch and a large diff; mitigated by the task order (O2), the
  test names kept (T10), and CI's full tier on the PR's merge queue run.
- Per-keystroke calls add an envelope and a reply versus `tilr.v1`'s fire-and-forget input;
  the reply replaces `InputAck`, which `tilr.v1` already sent, so the count is the same.
- 16 MiB `max_frame` below `tilr.v1`'s 64 MiB decompression cap: frames with large images are
  split (B9); one image over 16 MiB is not shown.
- Events are lossy by design; a client that misses `pane.exited` sees it in `pane.list` or the
  stream's `End`.
- `pane.copied` reaches more subscribers than `ClipboardWrite` did (T6), within their rights.
- Bearer tokens can be stolen by a process of the same user (it can read the state directory
  anyway); origin binding (W2) narrows web tokens; proof of possession is a non-goal.
- Up to 1 s of use after expiry (decision TTL).
- macOS Seatbelt is deprecated (S10); caught by the CI matrix, fails closed.
- Linux: `client` leaves UDP open; ABI 4-5 kernels lack signal scoping (warned per pane);
  `AF_UNIX` refusal breaks programs that need D-Bus or an IDE socket (README).
- `biscuit-auth`, a Landlock crate, `websockets` (Python) and `protobufjs` must pass
  `cargo deny` / pinning.
- Two more toolchains in CI (Deno, uv), pinned with SHA-256 like Zig and protoc.

## Migration

- **M1. Clients in this repo** move with the switch (T10); nothing in this repo speaks
  `tilr.v1` afterwards except B3's one refusal packet.
- **M2. Tilr.app** (another repository: the `server` plugin, tilr-mac) stays on the current
  release line `release/X.Y` with `tilr.v1`; it moves to the bus after this change, and the new
  minor line `release/X.(Y+1)` is cut (`make_rc tilrd --minor`) on the owner's word when the app
  is ready. No adapter from `tilr.proto` to the bus is planned (owner). Until then nothing of
  this change is released (nothing is released from `main`); CDN `latest` and `beta` keep the
  old line, so `tilrd update` of app users does not jump protocols.
- **M3. Servers**: an update across the switch needs a restart of tilrd (a reload into the new
  build is refused by `state-formats`; the old server keeps running), so shells of the old
  server end at that restart (Open Questions).
- **M4. Config**: `bus.toml` and `profiles/` are new and optional; first-round drafts' `calls`
  key is refused with a hint; the root key is made on first start.

## Open Questions

- **Shells across the switch (M3).** Options: (a) refuse the reload from a `tilr.v1` build
  (supervisor-state 2) into the bus build, as every format change so far; the update needs a
  restart and the old shells end; (b) let the bus build also read `supervisor-state 2` and
  write it back when the predecessor's checker asks — `check_formats` takes the first
  `supervisor-state` line (`crates/tilrd/src/reload.rs:95-110`), so the new build would print
  `supervisor-state 2` and lose the profile/creator data, and every later reload would need
  the same trick; (c) a one-time migration command (`tilrd migrate-state`) run by the update.
  Practice: tmux refuses to talk to a server of another protocol version and tells the user to
  restart it (`protocol version mismatch`); systemd re-exec keeps state only within a
  serialization format it reads. Rec.: (a), documented in the release notes of the new minor
  line; shells of an app user end once, at the switch.
- **Fixes for the old release line after the switch lands in `main` (M2).** `make_rc` merges
  a commit of `main` into the current `release/X.Y`, so after the switch a fix for the `tilr.v1`
  line cannot go upstream first without bringing the bus into it. Options: (a) merge the switch
  into `main` and allow fixes for the old line as PRs directly into `release/X.Y` (a named,
  temporary exception until the new minor ships); (b) keep the PR open until the app is ready
  (a long-lived branch, against trunk-based work); (c) cut the new minor line right after the
  merge and keep the old line frozen (no fixes). Practice: Go and Kubernetes keep release
  branches alive with backports that cannot be merges of `main`; Linux stable takes
  "upstream first" except when the code differs, then a direct stable patch. Rec.: (a).
