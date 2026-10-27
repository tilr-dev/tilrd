# Proposal: cloudflared never outlives the server or catches its terminal's signals

## Why

- The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #25): a stuck `cloudflared` outlives the exec (`cloud.rs:629` then). At 1163abd this half is already handled: `stop` sends SIGTERM, waits `TUNNEL_STOP` (3 s) and then SIGKILLs and reaps the child (`crates/tilrd/src/cloud.rs:686-699`), and the supervisor awaits that before the exec (`Tunnel::hold`, `cloud.rs:248-255`; `supervisor.rs:1554`, `1844`); the `reload` spec states it ("cloudflared is stopped before the exec", "killed if it does not exit in time") but no test makes `cloudflared` ignore SIGTERM.
- The `cloud-tunnels` requirement "cloudflared does not outlive the server" (SIGKILL scenario) is not met at 1163abd: `cloudflared` is a plain child with `kill_on_drop` only (`cloud.rs:539-549`), no parent-death signal, so a SIGKILLed server leaves it running; itests UC-088 (itests/USE_CASES.md group H) has no scenario. USE_CASES.md says so ("`cloudflared`, оставшийся после аварийной смерти сервера (SIGKILL), никто не убирает").
- Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`: `cloudflared` sits in the supervisor's process group, so a terminal's SIGHUP kills it and the server restarts it (tunnel flicker). At 1163abd the pane processes get a session of their own (`supervisor.rs:2883-2895`) and the execs a group of their own (`exec.rs:422`), but `cloudflared` neither.

## What Changes

- `cloudflared` is started in a process group of its own, so signals to the server's group (a terminal's SIGHUP or Ctrl-C) do not reach it.
- On Linux `cloudflared` is started with a parent-death signal (`PR_SET_PDEATHSIG` SIGTERM, so it signs off at Cloudflare), checked against a parent that died before it was set (design D1).
- Everywhere (and the only cover on macOS, which has no parent-death signal): the server records the running `cloudflared`'s pid in `<runtime dir>/cloudflared/pid` (0600); a starting server, and a supervisor resuming after a reload, stops a `cloudflared` left there that still runs with this runtime directory's `config.yml` before it starts its own (design D2).
- A test that a `cloudflared` ignoring SIGTERM is killed and does not hold back a reload (behavior already in the `reload` spec; only the test is new).
- itests UC-088 gets its scenario.

## Capabilities

### Modified Capabilities
- `cloud-tunnels`: "cloudflared does not outlive the server" gains the process group and the leftover sweep, and its tests.
- `reload`: "cloudflared is stopped before the exec" gains the SIGTERM-ignoring scenario and its test.

## Non-goals

- Cloud-side tunnel handling, or a real Cloudflare tunnel in tests.
- A watchdog process for `cloudflared` on macOS (the sweep at the next start is the cover there; design D2).
- Any `cloudflared` started by something other than this server.

## Impact

`crates/tilrd/src/cloud.rs` (spawn options, pid file, sweep), `crates/tilrd/tests/cloud.rs`, itests `uc_088.py` (new), README "tilr Cloud", USE_CASES.md, itests/USE_CASES.md (UC-088). No protocol change, no PROTOCOL_VERSION or feature change, no state file format change: the pid file is not reload state (the successor sweeps it as any start does). Survives a reload: the old supervisor still stops its `cloudflared` before the exec and the successor starts its own; the sweep finds nothing in the normal case. An update from the previous release is a reload into this one: the old image stops its `cloudflared` as before, and the first `cloudflared` started by the new image is the first one with a pid file. Not breaking.

Implemented by: [`0aff8e0`](https://github.com/tilr-dev/tilrd/commit/0aff8e0) fix(cloud): never leave cloudflared running after the server (2026-10-18).
