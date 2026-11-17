# Tasks

## 1. Built-in modules

- [ ] 1.1 `builtin` and `args` in `bus.toml` (D2), reload of built-in instances after a tilrd reload into a new executable; verify `gateway::the_gateway_runs_as_a_built_in_instance` and a `reload::` case for the executable change; README "Components"

## 2. Gateway

- [ ] 2.1 `tilrd gateway` on `hyper` 1.x (exact versions; `cargo deny check`), bind and Host/Origin rules (D3); verify `gateway::the_gateway_listens_on_loopback_only`, `gateway::browser_origins_are_refused_unless_allowed`
- [ ] 2.2 Bearer token forwarded as credential (D4), routes and mapping (D5); verify `gateway::every_http_call_carries_a_token`, `gateway::routes_and_methods`, `gateway::bodies_and_content_types_pass_through`, `gateway::bus_errors_become_http_statuses`; USE_CASES.md entries
- [ ] 2.3 Limits and logs (D6, D7); verify `gateway::the_gateway_bounds_what_it_accepts`, `gateway::the_gateway_does_not_log_secrets`
- [ ] 2.4 README "Components": the gateway (example config, `curl` with a token from `tilr bus token`, status table, why loopback only); verify by review against the `bus-gateway` spec

## 3. Checks and archive

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 Update the row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
