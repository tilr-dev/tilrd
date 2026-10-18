# Tasks

## 1. Token

- [ ] 1.1 Create the token (object read/write on `tilr-cdn` only) and store it in Infisical `/cloudflare` as `R2_CDN_TOKEN`; verify with it exported: `CLOUDFLARE_ACCOUNT_ID=… wrangler r2 object put tilr-cdn/test/scoped-token --file /dev/null --remote` succeeds, `wrangler r2 object delete tilr-cdn/test/scoped-token --remote` succeeds, and `wrangler r2 bucket list` fails (no account-wide access)
- [ ] 1.2 `infisical secrets get R2_CDN_TOKEN --path /cloudflare --plain --silent | gh secret set CLOUDFLARE_API_TOKEN --env cdn -R tilr-dev/tilrd`; verify `gh secret list --env cdn -R tilr-dev/tilrd` shows a new date
- [ ] 1.3 Comments in `.github/workflows/release.yml` (job `cdn`) and `scripts/release.sh` say the token is scoped to the bucket; the Basic Memory note's open item is closed; verify by review
- [ ] 1.4 The first release's `cdn` job (owner's step) uploads with it; verify its log ends with `published <version> for: linux-x86_64`

## 2. Checks and archive

- [ ] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 2.2 No itests case is touched; `./itests/dt coverage` exits 0
- [ ] 2.3 Record the commit in proposal.md and archive the change
