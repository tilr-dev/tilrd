# Proposal: Forwarding TCP services through the client connection

## Why

TCP service forwarding (Postgres, Redis) was a direction recorded on 2026-10-03; `ssh -L`-style forwarding was deferred when SOCKS shipped; the stream protocol was designed generic (`StreamOpen{kind}`, first kind `ssh-agent`, later reverse ports) (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, `main/projects/tilr/tilr-proksi-v-brauzere-socks-over-ws-cherez-core-profili-wkweb-view-izvestnye-ogranicheniia`, `main/projects/tilr/tilr-probros-ssh-agenta-sshagent-most-mac-app-stranitsa-tilr-server-kak-ssh-a`).

## What Changes

- To be designed: stream kinds `tcp-forward` (client asks the server to connect to host:port) and `tcp-reverse` (server listens and opens streams to the client), on the bulk lane.

## Capabilities

### Modified Capabilities
- `ssh-agent-forwarding`
- `socks-proxy`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Remote desktop over WebRTC (recorded direction, separate).

## Impact

Additive stream kinds and feature flags; listeners of reverse forwards must survive a reload like the agent sockets.
