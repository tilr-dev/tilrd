# Proposal: End-to-end encryption between client and server

Блокер: Which E2E scheme? Options: (a) Noise (XX/IK) inside the WebSocket with a server key pair in the state directory and keys pinned by the client on first use or via Cloud; (b) TLS with a self-signed server certificate pinned by the client (tunnel passes TCP through); (c) postpone and accept Cloudflare TLS termination for v1. Recommendation: (a), with pinning via the existing `server_id` and Cloud as the key directory; a note says you already worked it out — please link it.

## Why

End-to-end encryption was recorded as a requirement on 2026-10-03 with details deliberately left open ("обсудим позже"); Cloud v1 terminates TLS at Cloudflare, so the tunnel provider sees the traffic. One note says E2E "is already worked out" (Dan) but no design is recorded (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, `main/projects/tilr/tilr-cloud-v1-control-plane-tunneli-token-dostupa-realizatsiia`, `main/projects/tilr/tilr-cloud-besplatno-dlia-vsekh-premium-optsionalno-1`).

## What Changes

- To be designed: a key exchange bound to the server's identity (`server_id` plus a key pair) and the client, encrypting packets inside the WebSocket, so a tunnel sees only ciphertext.

## Capabilities

### Modified Capabilities
- `protocol`
- `access-control`
- `cloud-tunnels`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- TLS on the listen port itself.

## Impact

**BREAKING** or a feature flag depending on the design; key material in the state directory; a reload must keep session keys or force a re-handshake (clients reconnect on a reload anyway).
