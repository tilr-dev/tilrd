# Proposal: SSH agent forwarding through the client

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Signing git and ssh in a remote pane with the Mac's key (1Password), like `ssh -A`, without ssh (Basic Memory `main/projects/tilr/tilr-probros-ssh-agenta-sshagent-most-mac-app-stranitsa-tilr-server-kak-ssh-a`). On 2026-10-15 Dan asked that forwarding be a configurable option, not hardcoded (`main/projects/tilr/tilr-zhurnal-reshenii`).

## What Changes

- Every pane has `SSH_AUTH_SOCK` pointing at `agent/<pane>.sock` in the runtime directory; a connection opens a stream to the client that subscribed last with `agent` in `Hello.features`.
- Streams in the protocol: `StreamOpen{stream_id, kind, pane_id}`, `StreamData`, `StreamClose` both ways; the server carries bytes only.
- Sockets come back after a reload; off with `--no-agent` or `agent = false`.
- `tests/agent.rs`.

## Capabilities

### Modified Capabilities
- `ssh-agent-forwarding`

## Out of scope

- Other stream kinds such as reverse ports (open change `port-forwarding`).

## Impact

Implemented by: [`e842b1b`](https://github.com/tilr-dev/tilrd/commit/e842b1b) SSH agent forwarding, as ssh -A … (2026-10-10).
