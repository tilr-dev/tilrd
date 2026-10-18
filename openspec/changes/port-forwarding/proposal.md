# Proposal: Forwarding TCP ports through the client connection

Deferred: after the transport decision (iroh vs quinn; see `quic-transport`, `directory-sync`).

Зависит от: protocol-v3

## Why

TCP service forwarding (Postgres, Redis) was a direction recorded on 2026-10-03; `ssh -L`-style forwarding was deferred when SOCKS shipped; the stream protocol was designed generic (`StreamOpen{kind}`, first kind `ssh-agent`, later reverse ports) (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, `main/projects/tilr/tilr-proksi-v-brauzere-socks-over-ws-cherez-core-profili-wkweb-view-izvestnye-ogranicheniia`, `main/projects/tilr/tilr-probros-ssh-agenta-sshagent-most-mac-app-stranitsa-tilr-server-kak-ssh-a`). The owner is researching iroh as the transport for everything (Basic Memory `main/projects/tilr/tilr-iroh-kak-transport-dlia-vsego-vopros-na-prorabotku`, 2026-10-18).

Checked at 1163abd: streams exist only server-initiated with kind `ssh-agent` (`crates/tilrd/src/agent.rs`, `tilr.proto` "Streams"); the SOCKS proxy (`crates/tilrd/src/socks.rs`, off unless `--socks`) already lets a client open TCP connections from the server's machine (CONNECT on `/socks`); nothing lets the server listen for a client; `tilr` has no forwarding command (`crates/tilr/src/main.rs:52-130`).

## What Changes

- **Remote to local (`ssh -R`)**: new `ForwardListen { request_id, port }` asks the server to listen on `127.0.0.1:<port>` (0 = any free port); `ForwardListening { request_id, forward_id, port, error }` answers. Each connection to it opens a stream to that client, `StreamOpen { kind: "tcp-forward", forward_id }`, then `StreamData` / `StreamClose` as for the agent. `ForwardStop { forward_id }` ends it; a forward also ends with its client's connection.
- **Local to remote (`ssh -L`)**: no server change; `tilr forward -L [ADDR:]LPORT:HOST:HPORT` listens on the user's machine and carries each connection over the server's SOCKS proxy (`/socks` with the token), so it needs `--socks` on the server and the TCP port.
- `tilr forward -R PORT:HOST:HPORT` uses `ForwardListen`; both forms run until interrupted and reconnect (re-asking their forwards) like `tilr attach`.
- Feature `"forward"` in `HelloAck.features` (a capability the server can be run without: `--no-forward` / `forward = false`, or off by default per the Decision); a client lists `"forward"` in `Hello.features` to be sent `tcp-forward` streams.
- Limits: at most 16 forwards per client; streams count toward the existing 64 per client; data on the bulk lane with the agent streams' backpressure rules.

## Capabilities

### New Capabilities
- `port-forwarding`: see `specs/port-forwarding/spec.md`.

## Non-goals

- Listening on anything but the server's loopback (no `0.0.0.0` forwards, no `GatewayPorts`).
- Forwarding UDP; forwarding Unix sockets.
- Keeping reverse listeners across a reload (they end with the connection; `tilr` re-asks after it reconnects).
- A QUIC/iroh transport (changes `quic-transport`, `directory-sync`).
- Remote desktop over WebRTC (recorded direction, separate).

## Impact

`crates/tilr-proto/proto/tilr.proto` (`ForwardListen`, `ForwardListening`, `ForwardStop`, `StreamOpen.forward_id`, feature `"forward"`; additive), `crates/tilrd/src/{forward.rs (new), supervisor, config, main}.rs`, `crates/tilr/src/{main,forward.rs (new)}.rs`, README new section "Port forwarding" and "Protocol", USE_CASES.md, itests UC-07E. Reload: nothing crosses it; listeners close at the exec with every connection and `tilr` re-asks for them when it reconnects (a connection attempt in the gap is refused). Update from the previous release: works; the feature appears after the update.

Not breaking: additive messages and fields only, no PROTOCOL_VERSION bump, no close code, state file or CDN change.

## Decision (2026-10-18)

Wait for the transport decision (option (c)): whether QUIC comes from iroh or quinn (`quic-transport`, `directory-sync`; Basic Memory note "Tilr — iroh как транспорт для всего") shapes how each forwarded connection is carried. Not built before it.

Prepared plan, not a decision: when built, the server side is off by default like SOCKS (`--forward` / `forward = true`), since a loopback TCP port is open to every user of the server's machine. The messages are transport-neutral protobuf and would move to QUIC streams unchanged.
