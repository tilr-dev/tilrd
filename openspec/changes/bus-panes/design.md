# Design: Panes on the bus

## Context

After `pane-profiles`, `component-bus` and `bus-acl-biscuit`: `CreatePane.profile` with the
profile snapshot kept across reloads; a broker in tilrd with control methods answered by tilrd
itself; every call authorized by a Biscuit (connection token or per-call credential) before
routing, with authorizer facts `caller`, `method`, `target`, `time`, `server`. The supervisor
already does everything a pane needs (creation, input with acknowledgements, resize, close,
frames from the pane process's screen model, reload handover).

Prototype `term` (see proposal): a separate runtime, panes in memory, no emulator, long-poll on a
byte cursor (`term.read {since, wait_ms}` capped at 4 s by `TERM_MAX_WAIT_MS`, below the 5 s call
timeout), SIGHUP then SIGKILL on close.

## Goals / Non-goals

Goals: a runtime (an agent orchestrator, a script, the gateway's callers) creates and drives panes
with profiles, with least privilege, without a second pane implementation. Non-goals: see
proposal.

## Decisions

### D1. tilrd is the provider
The supervisor registers `pane.*` under the reserved instance name `tilrd` (always `up`, never
drained, not in `bus.toml`). Calls are routed to it like to any instance; it answers in-process.
Rejected: a separate `term`-like runtime talking to tilrd over the client protocol (two
authorization models, one more hop; the prototype's choice only because it had no tilrd).

### D2. Methods and mapping (JSON bodies)
| method | body | answer | maps to |
|---|---|---|---|
| `pane.create` | `profile`, `command?`, `cwd?`, `cols?` (80), `rows?` (24) | `PaneInfo` as JSON | the `CreatePane` path; errors `create pane: …` → `bad_request` (or `unavailable` when the pane process died) with the same message |
| `pane.list` | — | `{panes: [PaneInfo]}` (only those the caller may access) | `ListPanes` |
| `pane.write` | `pane_id`, exactly one of `text` / `data_b64` | `{written}` after the bytes reached the pty | the `RawInput` path with its acknowledgement |
| `pane.screen` | `pane_id`, `since_seq?`, `wait_ms?` (≤ 25000), `history_lines?` (≤ 10000) | `{seq, cols, rows, lines: [text], cursor: {x, y, visible}, title, cwd, alive, exit_code?, history: [text]}` | a text snapshot from the pane process's screen model |
| `pane.resize` | `pane_id`, `cols`, `rows` | `{}` | `Resize` |
| `pane.close` | `pane_id` | `{exit_code}` once the program ended | `ClosePane` |

Unknown fields → `bad_request`; unknown pane → `not_found` `no such pane: <id>`.

### D3. Output without a second emulator
`pane.screen` asks the pane process for a text rendering of its screen (each row's cells as text,
trailing blanks trimmed; wide characters once) and of up to `history_lines` scrollback lines; `seq`
is the pane's frame number. With `since_seq` = the last `seq` seen and `wait_ms`, the answer comes
when the screen changes after it, the program exits, or `wait_ms` passes (then the same `seq`).
This replaces the prototype's byte ring and byte cursor: what an agent needs is what the user
sees, and tilrd already computes it. The wait is `min(wait_ms, 25 s, time the call has left −
100 ms)`, so it always answers before the call's deadline (the prototype needed
`TERM_MAX_WAIT_MS` kept below `call_timeout_ms` by hand). Rejected: raw output bytes (needs a byte log tilrd does
not keep; replays escape sequences an agent must emulate again); pushing frames over the bus (no
streams until the direct channel, `component-bus` D3).

### D4. Rights
The broker checks `call("pane.create")` etc. before routing; it cannot see payloads. tilrd as the
provider then authorizes again with the same token (the broker passes the verified token
in-process) and these facts: `pane_profile_requested(<name or "">)`, `pane_overrides(<bool>)`,
`pane(<id>)`, `pane_creator(<creator>)`, `caller(<identity>)`. Rights in the token:

- `pane_profile("agent")` — may create panes with that profile; `pane_profile_any()` — any profile;
  `pane_unrestricted()` — may create a pane without a profile (an unrestricted shell: owner-level);
- `pane_override()` — may set `command`/`cwd` over the profile's (`pane-profiles` D5 left this to
  rights);
- `pane_access("own")` — write/screen/resize/close/list only panes whose `created_by` is
  `bus:<caller>`; `pane_access("any")` — every pane, including the owner's.

Config: `[instances.<n>.rights] pane = ["profile:agent", "access:own"]`, minted into the
instance's token. `tilr bus token --pane profile:agent --pane access:own` for outside callers.
`owner` holds all. A denial is `denied` with the missing right named, audited as any denial.

### D5. Creator
`PaneInfo.created_by` = `owner` for panes from tilrd clients, `bus:<instance>` for panes from the
bus (the caller's identity, also when a per-call credential was used: the credential limits, the
caller owns). It is set once and handed over in `PaneState.info`; `supervisor-state` goes to 4 so
an older successor (which would drop it and so break `access:own`) is not reloaded into.

### D6. Lifetime
Bus-created panes outlive their creator, as panes outlive clients; a stopped or crashed instance
does not close its panes. Its next process, being the same instance name, can access them again
with `access:own`.

## Reload and reconnect

Panes cross tilrd's reload as always (with `created_by`). A `pane.screen` waiting at the exec is
answered `unavailable` by the broker's drain (`component-bus` D13); the caller retries with its
last `seq`. `pane.write` already acknowledged is not repeated; one not yet acknowledged at the
exec is answered `unavailable` and the caller cannot know whether the bytes reached the pty
(documented; the same holds for a client's `RawInput` without input identity).

## Backpressure

Answers are bounded: a screen of 65535 × 65535 is clamped by the pane's real size; `history_lines`
≤ 10000 and the answer ≤ `max_frame` (else `too_large`, ask for fewer lines). `pane.write` body ≤
1 MiB per call (else `bad_request`), written through the pane's input path, which already waits
for the pty.

## Risks / Trade-offs

- Long-poll `pane.screen` keeps a call open up to its wait; counted in the caller's in-flight
  limit.
- `pane_unrestricted` and `access:any` are owner-level; README says so.
- A text snapshot loses colours and attributes (not needed by agents now; a `styled` option can be
  added).

## Open Questions

- D3 screen text + long-poll on `seq` instead of a raw byte stream. Rec.: yes until the direct
  channel exists (tmux `capture-pane` + `wait-for` serve automation the same way).
- D4 `pane_access` default `own`. Rec.: `own` (Docker's per-container permissions, Kubernetes
  owner references).
- D6 panes outlive their creating instance. Rec.: yes (like panes outlive clients); a
  `close_with_creator` option later if wanted.
- Whether `pane.*` names should stay `term.*` for prototype compatibility. Rec.: `pane.*` (the
  product's word; no prototype users).
