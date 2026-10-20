# Cause and fix

On 2026-10-20 the previous session reproduced RUSTSEC-2026-0382 for
rustls-webpki 0.103.15 and RUSTSEC-2026-0381 for webpki-roots 0.26.11 / 1.0.9.
Updating the lock file fixes rustls-webpki and roots 1.x but leaves the
tokio-tungstenite bundled-roots feature's 0.26 dependency.

Use tokio-tungstenite's rustls transport feature with an explicit ClientConfig
whose root store is exactly webpki-roots 1.0.10's TLS_SERVER_ROOTS. Both Cloud
and ps use the same helper. Retain default WebSocket configuration, Nagle
behavior and no client certificate. Native roots and advisory ignores are
not alternatives permitted by Dan's #4202 decision.

The lock-file regression fails on the original affected graph. Loopback tests
use an ephemeral CA and key generated in memory, so TLS success, an untrusted
issuer and a hostname mismatch require no public service or clock sleeps.
