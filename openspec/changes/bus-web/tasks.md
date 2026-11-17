# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Tilr Web

- [ ] 1.1 [9.1] Origin on the bus handshake, the `origin` fact of `bus-rights` checked end to end, `LAST_ACTIVE` providers from pages, identity by token name (W1-W4); verify `bus_web::a_page_calls_subscribes_and_provides`, `bus_web::only_allowed_origins_connect`, `bus_web::a_token_bound_to_an_origin_works_only_there`, `bus_web::a_closed_window_makes_its_methods_unavailable`, `bus_web::two_windows_one_provider`, `bus_web::a_page_keeps_its_identity_and_renews_its_token` (reconnect with a renewed token, renewal on the open connection; no reload); README "The bus": Tilr Web (how the app mints plugin tokens); USE_CASES.md entries (own section)

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
