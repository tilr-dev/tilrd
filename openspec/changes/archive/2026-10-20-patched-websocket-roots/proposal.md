# Use patched bundled WebSocket trust roots

## Why

TILR-244: the bundled-roots feature of tokio-tungstenite 0.30 pulls in
webpki-roots 0.26.11, affected by RUSTSEC-2026-0381 even after its 1.x
dependency is updated. Dan approved an explicit rustls connector (#4202).

## What Changes

- Keep tilrd, tilr and tilr-driver's outgoing WebSocket connections on bundled Mozilla roots via
  webpki-roots 1.0.10 directly, with normal certificate and hostname verification.
- Remove the affected 0.26 dependency without advisory ignores or native roots.
- Cover the dependency graph, trust roots and local WebSocket TLS handshakes.

## Capabilities

- Add a release-process requirement for tilrd's bundled WebSocket trust roots.

## Impact

Cargo manifests/lock, the shared Cloud, ps, tilr and tilr-driver connector, regression tests.
No command, protocol or trust-store change.
