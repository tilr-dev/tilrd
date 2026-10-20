## 1. Bundled trust roots

- [x] 1.1 Specify the preserved WebSocket trust policy and record the advisory cause.
- [x] 1.2 Replace the implicit bundled-roots connector and remove affected dependencies.
- [x] 1.3 Add deterministic regression coverage and verify it fails without the fix.
- [x] 1.4 Run self-review and the required checks before Review.

## 2. Review follow-up

- [x] 2.1 Share the explicit connector with tilr and tilr-driver.
- [x] 2.2 Verify client TLS regression and date the upstream workaround.
- [x] 2.3 Run self-review and checks again.

## 3. Bundled-anchor regression

- [x] 3.1 Reproduce the old tests staying green with an empty production store.
- [x] 3.2 Cover the public connector and both client entry points with a local matching-issuer peer.
- [x] 3.3 Verify empty-store and both implicit-client mutations fail with UnknownIssuer.
- [x] 3.4 Remove the empty manifest section and clarify the affected callers in the proposal/design.
