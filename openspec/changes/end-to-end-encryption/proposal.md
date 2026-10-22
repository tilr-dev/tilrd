# Proposal: End-to-end encryption between client and server

Deferred: security design to be done.

## Why

End-to-end encryption was recorded as a requirement on 2026-10-03 with details deliberately left open ("обсудим позже"); Cloud v1 terminates TLS at Cloudflare, so the tunnel provider sees the traffic. One note says E2E "is already worked out" (Dan) but no design is recorded (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, `main/projects/tilr/tilr-cloud-v1-control-plane-tunneli-token-dostupa-realizatsiia`, `main/projects/tilr/tilr-cloud-besplatno-dlia-vsekh-premium-optsionalno-1`).

## Decision (2026-10-18)

- Not now: v1 relies on TLS of the WSS connection, terminated at Cloudflare.
- This change becomes a security design task: work out how to do E2E properly, then propose the implementation as its own change.
- Rejected: implementing a scheme now (no recorded design to build on).

## What Changes

- No code. A design document (design.md, to be written) covering:
  - Threat model: Cloudflare (the tunnel) and Tilr Cloud must not read or alter session traffic; the client and the server machine are trusted; the local unix socket is excluded (same user, same machine).
  - Candidate approaches, compared neutrally: Noise (XX / IK) inside the WebSocket with a server key pair in the state directory and the key pinned by the client; TLS end to end with a pinned self-signed server certificate (the tunnel passes TCP through); others found while designing.
  - Key distribution and pinning (first use, `server_id`, Cloud as a directory without trusting it), rotation, multiple clients, and what a reload and a reconnect do to session keys.
  - Protocol impact per the protocol version policy in `openspec/config.yaml`: version 1 without a bump before the first public release; afterward, bump only for incompatibility with a client or server of the previous public release, once per release as a separate step in its release milestone.

## Capabilities

### Modified Capabilities
- `protocol`, `access-control`, `cloud-tunnels`: none until the design is decided.

## Non-goals

- Implementation in this change.
- TLS on the listen port itself; the local unix socket.

## Impact

None now. The later implementation will likely be **BREAKING** and keep key material in the state directory. Under the protocol version policy in `openspec/config.yaml`, even an incompatible change uses version 1 without a bump before the first public release; afterward, a bump is a separate step in its release milestone, once per release, only for incompatibility with a client or server of the previous public release.
