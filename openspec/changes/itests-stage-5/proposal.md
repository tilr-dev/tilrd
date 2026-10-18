# Proposal: itests stage 5: load and security group H, distributions, a macOS backend

Зависит от: test-coverage-gaps

## Why

itests/USE_CASES.md group H (UC-080 … UC-087) has no scenarios yet; thresholds are to be measured first and set at about 2x headroom after 3-5 runs on pb. Planned platforms are Ubuntu LTS, Debian, Fedora, Alpine (musl) and aarch64, and a `local` backend running the same scenarios as processes on macOS (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- Scenarios UC-080 (200 panes, 50 clients), UC-081 (output storm), UC-082 (huge Resize), UC-083 (64 MB message), UC-084 (token guessing logged), UC-085 (foreign Origin, DNS rebinding on `/metrics` and `/socks`), UC-087 (24 h soak); UC-086 and UC-088 belong to `sighup-ignoring-shell-reaped` and `cloudflared-outlives-exec`.
- Thresholds from measured runs.
- Distribution matrix and the `local` backend (BLOCKED for container-only cases).

## Capabilities

### Modified Capabilities
- `metrics`
- `client-liveness`
- `access-control`
- `panes`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Out of scope

- Fixing what the new cases find (own changes).

## Impact

`itests/` only. No behaviour change.
