# Design: Component bus in tilrd

## Context

Prototype (`runtime-tmp`, commit 9fbbc3e per its docs): `tilr-host` = supervisors + broker +
controller. Frame = u32 big-endian length + protobuf `Envelope{Hello|HelloAck|Call|Reply}`,
16 MiB inbound cap (`wire.rs`). Broker (`broker.rs`): one `Mutex<State>` (instances, round-robin
counters, `pending`, `next_id`), ACL in a `RwLock`, unbounded per-connection queues, a timer task
per call that is never cancelled; errors `denied`, `not_found`, `unavailable`, `timeout`,
`bad_reply`, plus controller and SDK codes. Controller (`control.rs`): `bus.reload`
(`instance|module|all`, rolling, drain), `bus.reload_config` (diff), `bus.start`, `bus.stop`,
one lock for all of them, `SELF_GRACE` 300 ms. Runner (`main.rs`): `process_group(0)`, stdin
`/dev/null`, output prefixed `[name]`, env `TILR_INSTANCE`, `TILR_CONFIG_DIR`, `TILR_CONFIG`,
`TILR_BUS_FD=3` or `TILR_BUS_SOCK`+`TILR_BUS_TOKEN` (one-time), backoff 200 ms doubling to 5 s,
reset after 10 s up; stop = SIGTERM to the group, 3 s, SIGKILL. Config `bus.yaml` (YAML,
`deny_unknown_fields`).

tilrd at 14388b16: one supervisor process with the client protocol (WebSocket over TCP and the
Unix socket), pane processes as children, reload by exec in place with `SupervisorState` handed
over through `server-<pid>.state` (format 2; 3 after `pane-profiles`), runtime directory
`$TILR_RUNTIME_DIR` / `$XDG_RUNTIME_DIR/tilr` / `<tmp>/tilr-<euid>`, configuration directory as
in `pane-profiles` D1.

## Goals / Non-goals

Goals: the prototype's model in product quality inside tilrd; its stated limits either fixed or
named as non-goals (table below); a path for the owner to call the bus by hand; survive tilrd's
reload.

Non-goals: see proposal.

## Decisions

### D1. Crates
`crates/tilr-bus`: `proto/bus.proto`, framing codec, limits, error codes, and an async client
(`Bus::connect_from_env`, `register`, `call`, `serve`) for Rust components and tests; no broker.
The broker, runner and controller are a module of tilrd (`crates/tilrd/src/bus/`), because the
broker lives in tilrd (2026-10-26) and shares its reload. A test runtime
`crates/tilr-bus/examples/test_runtime.rs` (echo, sleep, fail, nested-call methods; never
shipped) backs the integration tests. Rejected: a `tilr-bus-broker` crate (no second user; tilrd
is the only host); a separate `tilr-runner` binary (one more process to reload and supervise).

### D2. Topology and routing (kept from the prototype)
Star; the identity of a connection is the instance it was created for; `from` is always set by
the broker; `target` names an instance or is empty (round-robin over live, non-draining
providers in config order); ids remapped per connection; a reply is accepted only from the
callee's connection (else dropped as late or forged). Methods are `<namespace>.<name>`
(`[a-z0-9_-]` segments, at least two); `bus.*` is answered by tilrd and cannot be provided.

### D3. Wire protocol `tilr.bus.v1`
Framing unchanged: u32 BE length + `Envelope`. Changes:

```proto
message Envelope { oneof msg { Hello hello = 1; HelloAck hello_ack = 2; Call call = 3;
                               Reply reply = 4; Cancel cancel = 5; } }
message Hello    { string token = 1; repeated string provides = 2; uint32 protocol_version = 3;
                   bytes credential = 4; /* reserved for bus-acl-biscuit */ }
message HelloAck { string instance = 1; uint32 protocol_version = 2; uint32 max_frame = 3;
                   uint32 max_in_flight = 4; }
message Call     { uint64 id = 1; string target = 2; string method = 3; bytes payload = 4;
                   string from = 5; string trace_id = 6; uint32 timeout_ms = 7;
                   string content_type = 8; bytes credential = 9; /* bus-acl-biscuit */ }
message Reply    { uint64 id = 1; oneof result { bytes payload = 2; Error error = 3; }
                   string content_type = 4; }
message Cancel   { uint64 id = 1; string reason = 2; }
message Error    { string code = 1; string message = 2; }
```

- `protocol_version` 1; a broker answers another version with `Reply`-less close after logging
  (the instance is treated as failed to start).
- `max_frame` 16 MiB, enforced on both directions (the prototype checked inbound only); a call
  larger than it is answered `too_large` by the sender's SDK, a frame larger is a protocol error
  that closes the connection.
- `timeout_ms`: what the caller allows; the broker uses `min(timeout_ms, call_timeout_ms)` and
  forwards the remaining time, so a nested call cannot outlive its parent's deadline (fixes "no
  deadline down the chain").
- `Cancel`: sent by the broker to the callee when a call times out or its caller goes away;
  advisory (the callee may stop work; a later reply is dropped). Fixes "no cancellation" as far
  as a bus can.
- `content_type`: opaque to the broker; carried so a gateway can set HTTP `Content-Type`
  (the prototype always said JSON). Payload bytes are never parsed by the broker except the JSON
  bodies of `bus.*` control methods.
- Unknown `Envelope` variants and fields are ignored by both sides: a future `ChannelGrant`
  (direct channel, 2026-10-26) is additive.
- Error codes (strings, set closed for the broker, open for handlers): `denied`, `not_found`,
  `unavailable`, `timeout`, `busy`, `too_large`, `bad_request`, `handler_error`, `bad_reply`,
  `not_supported`, `spawn_failed`, `start_failed`, `start_timeout`, `bad_config`.

### D4. Backpressure
Per connection an outbound queue bounded at 64 MiB and 4096 frames; per caller at most
`max_in_flight` (256) calls pending. A call to a callee whose queue is full, or over the caller's
in-flight limit, is answered `busy` at once (prototype: unbounded). Replies and `Cancel` are
always queued; a connection whose queue passes the bound with replies alone is closed as too
slow, which restarts its instance (D7). Bounds memory per connection.

### D5. No global lock
Routing table (instances, provides, states) behind a read-mostly lock; pending calls kept per
callee connection; ids from an atomic counter; per-call timers in one timer wheel, removed on
reply. Observable: a slow or blocked instance does not delay calls between other instances.

### D6. Config: `bus.toml`, its own file
`<config dir>/bus.toml`, strict TOML (`deny_unknown_fields`), read at start and by
`bus.reload_config`. Its own file because it is reloaded live while `server.toml` is read only
at start (README "The configuration file"); one file with two reload rules would surprise.
TOML rather than the prototype's YAML: one config language in tilr (`server.toml`, profiles).

```toml
call_timeout_ms = 10000      # default 10000
drain_ms = 3000              # default 3000
[modules.deno]
command = ["deno", "run", "-A", "runtimes/deno/main.ts"]   # required, non-empty
cwd = "~/tilr/components"    # absolute or ~; default the config dir
[modules.deno.env]
LOG = "info"
[instances.deno-1]
module = "deno"              # required
connect = "fd"               # "fd" (default) | "socket"
autostart = true             # default true
calls = ["math.*"]           # minimal rule (D10); replaced in bus-acl-biscuit
[instances.deno-1.env]
WORKERS = "2"
```

Instance names as profile names (`pane-profiles` D2); `owner`, `tilrd` and names starting with
`bus` are reserved. A config error at start is logged and the bus starts with no instances (tilrd
itself keeps serving panes; the prototype panicked); at `bus.reload_config` it is `bad_config`
with nothing changed.

### D7. Spawn and environment contract
`setsid` (own session and group: terminal signals and tilrd's own group stay apart), stdin
`/dev/null`, stdout/stderr read line by line into tilrd's log as `bus <name>: <line>`, the last
40 stderr lines kept for `start_failed`. Every other descriptor of tilrd is close-on-exec.
Environment: tilrd's own (minus the pane variables) + module `env` + instance `env` +
`TILR_INSTANCE=<name>`, `TILR_CONFIG_DIR=<config dir>`, and per transport:

- `fd`: socketpair end as fd 3, `TILR_BUS_FD=3`. No token: holding the fd is the identity.
- `socket`: tilrd listens on `<runtime dir>/bus/<name>.sock` (directory 0700, socket 0600),
  `TILR_BUS_SOCK=<path>`, `TILR_BUS_TOKEN=<128-bit hex>`, made per process start, accepted for
  one Hello only, compared in constant time and consumed under the same lock (fixes the
  prototype's check-then-rotate race). Other connections are closed after a 10 s Hello deadline.
  For components whose launcher closes inherited descriptors.

`sun_path`: the socket path is checked at config load against the platform limit (104 bytes
macOS, 108 Linux); too long = config error naming the path (prototype: endless backoff or panic).

A connection lost while the process lives: the runner kills the process group and restarts it
with backoff (an instance without its connection has no identity; the fd cannot be re-made).
This is the "no reconnect" limit resolved by policy, not by a reconnect protocol.

### D8. Supervision
Backoff 200 ms doubling to 5 s, reset after 10 s up; reload/start skip the backoff; Hello within
30 s of spawn (else `start_timeout`, kill, backoff); stop = SIGTERM to the group, 3 s, SIGKILL to
the group. States `down`, `starting`, `up`, `draining`, `stopped` (sticky until `bus.start` or
removal), as in the prototype, reported by `bus.instances`.

### D9. Control methods (answered by tilrd; JSON bodies)
`bus.instances`, `bus.methods` (live, non-draining providers only — prototype listed draining
ones), `bus.start {instance}`, `bus.stop {instance, drain_ms?}`, `bus.reload {instance | module |
all, drain_ms?}` (rolling: one instance at a time; drain, restart, wait for Hello),
`bus.reload_config` (diff as the prototype: removed → stopped, added → started, changed →
reloaded, manually stopped stays stopped). Operations are serialized (one controller queue, as
in the prototype; named as accepted). A caller that reloads or stops its own instance gets its
reply first; the action starts once that reply has been written to its connection (replaces the
300 ms `SELF_GRACE`).

### D10. Minimal authorization (until bus-acl-biscuit)
Default deny. An instance may call a method that one of its `calls` globs matches (`*` = any
run of characters, as the prototype); `bus.*` only if listed. The principal `owner` — a greeted
tilrd client with full access (Unix socket, or TCP with the token) — may call every method. A
denied call is `denied` with `<from> may not call <method>` before routing.

### D11. The owner calls by hand: through tilrd, not attach instances
The prototype's attach instances were a demo deviation (§11); product needs "the owner calls
runtimes by hand" and tests need to call methods. Both go through tilrd's client protocol, which
already authenticates the user: `BusCall{request_id, target, method, payload, content_type,
timeout_ms}` (ClientMessage 33) → `BusReply{request_id, payload | error{code, message},
content_type}` (ServerMessage 26), `from` = `owner`. `tilr bus call METHOD [PAYLOAD]` (payload
from the argument, `-` for stdin, `--file`; `--target`, `--timeout`), `tilr bus ls`,
`tilr bus start|stop|reload NAME`, `tilr bus reload --all`, `tilr bus reload-config`. Exit 0 on a
payload (written raw to stdout), 1 with `<code>: <message>` on stderr for a bus error.
Rejected: attach instances (a token file any process of the user can read takes over an
identity; races; no process to supervise). A component developer runs a component as an
instance whose module command is their dev command and uses `tilr bus reload NAME`.

### D12. No feature flag for the bus
The bus cannot be turned off: without `bus.toml` there are no instances, but `bus.*` answers.
Per the repo rule, no flag. An older server ignores `BusCall`; `tilr bus` gives up after its
timeout with `no answer from the server (is it older than the bus?)`. Listed as open question.

### D13. tilrd reload
Before exec: the broker stops routing new calls (answered `unavailable`, `tilrd is reloading`),
waits up to 1 s for calls in flight, answers the rest `unavailable` and sends `Cancel` to their
callees. `SupervisorState` gets a `BusState` (format `bus-state 1`): per instance name, module,
pid, state, connection fd (not close-on-exec across the exec), listener fd for `socket`,
provides, restart counters, a consumed-token flag. The successor reads `bus.toml` again
(changes apply as a `bus.reload_config` diff), adopts the processes (still its children: same
pid) and resumes. Frames sent by instances during the exec wait in the socket buffers. Clients
reconnect as after any reload; a `BusCall` in flight is lost with the connection (the client
reports the disconnect; not retried, calls are not idempotent).

## Prototype limits: fixed or not

| limit (how-it-works §10) | here |
|---|---|
| no backpressure | fixed (D4) |
| one broker mutex | fixed (D5) |
| no direct channel | non-goal; protocol leaves room (D3) |
| no cancellation | `Cancel` + deadline (D3); callee may ignore |
| no dynamic registration | non-goal |
| ACL without delegation | minimal here (D10); `bus-acl-biscuit` |
| no reconnect | restart policy (D7) |
| `sun_path` | checked (D7) |
| one reload lock | kept, documented (D9) |
| timing heuristics | self-reload fixed (D9); 3 s / 30 s / backoff kept as constants |
| state after instance reload | non-goal |
| attach-token race | no attach instances (D11); socket token consumed atomically (D7) |
| no Hello timeout | 10 s (D7) |
| `bus.methods` lists draining | fixed (D9) |
| `term` limits | `bus-panes` |

## Reload and reconnect

D13. Instance reloads are the bus's own (D9) and do not touch panes or clients.

## Backpressure

D4 for the bus. On the client protocol, `BusReply` payloads up to 16 MiB go on the bulk lane of
the client outbox, bounded as other bulk traffic; a client too slow to drain is dropped (4001)
as today.

## Risks / Trade-offs

- Restart on lost connection (D7) kills a process whose connection broke for any reason; for the
  fd transport only a bug breaks it.
- Live-reloaded `bus.toml` next to start-only `server.toml` must be documented clearly.
- `owner` may call everything until `bus-acl-biscuit`; equal to today's token (it already gives
  shells).

## Open Questions

- D6: own `bus.toml` vs `[bus]` tables in `server.toml`. Rec.: own file (live reload vs start-only;
  systemd keeps units apart from `system.conf` the same way).
- D11: attach instances dropped; the owner uses `tilr bus` through tilrd. Rec.: drop them.
- D12: no `bus` feature flag. Rec.: no flag (repo rule).
- D7: a lost connection restarts the instance. Rec.: yes (systemd restarts a service whose socket
  activation fails the same way; NATS clients reconnect, but they hold credentials, ours is the fd).
- D13: instances survive tilrd's reload (handover) vs restart them on every reload. Rec.:
  survive, as panes do.
