# Proposal: A token scoped to the tilr-cdn bucket for the cdn environment

## Why

- The `cdn` GitHub environment of `tilr-dev/tilrd` holds secret `CLOUDFLARE_API_TOKEN` (set 2026-10-16) and variable `CLOUDFLARE_ACCOUNT_ID` (checked with `gh secret list --env cdn` / `gh variable list --env cdn` on 2026-10-18). The secret is a copy of Infisical `/cloudflare` `R2_TOKEN`, Admin read/write on all R2 of the account; a token scoped to the `tilr-cdn` bucket is an open item (Basic Memory `main/projects/tilr/tilrd-repozitorii-servera-na-git-hub-tilr-dev-tilrd-standart-org-ci-relizy-cdn`, "Следующие шаги" 2 and "Открытые хвосты").
- The only use is the `cdn` job of `.github/workflows/release.yml:186-229` at 1163abd, which runs `scripts/release.sh --upload-only` → `wrangler r2 object put tilr-cdn/<key> --remote` (`scripts/release.sh:112-123`); the environment is restricted to `main` by a branch policy.
- No release has run the `cdn` job yet (`https://cdn.tilr.dev/releases/linux-x86_64/latest` is 404 on 2026-10-18), so the token can be swapped before it is ever used.

## What Changes

- Create a Cloudflare API token with only object read/write on bucket `tilr-cdn` (permission "Workers R2 Storage Bucket Item Write", resource: that bucket), no expiry beyond the owner's rotation habit; store it in Infisical `/cloudflare` as `R2_CDN_TOKEN`; set it as `CLOUDFLARE_API_TOKEN` of the `cdn` environment.
- The secret's name and `scripts/release.sh` stay as they are; the comments in `release.yml:31-34` and `scripts/release.sh:35-40` say the token is scoped to the bucket.
- `R2_TOKEN` itself is not revoked: other tools of the account may use it; it just leaves this repository's environment.

## Capabilities

### Modified Capabilities
- `release-process`

No spec delta: infrastructure only (the requirement "The CDN gets the release last" names the secret, which keeps its name); `skip_specs: true`.

## Non-goals

- Tokens of other repositories or of the `tilr.sh` Worker (deployed by hand with `wrangler login`).
- Moving the upload off wrangler (e.g. to the S3 API with R2 access keys).

## Impact

GitHub environment `cdn`, Infisical `/cloudflare`, comments in `.github/workflows/release.yml` and `scripts/release.sh`. No code path, protocol, reload or update change. If `installer-atomic-install` lands as recommended, the same job also uploads `install.sh` to the same bucket, which the scoped token covers. Creating the token needs the owner's Cloudflare dashboard (or Infisical `/cloudflare` access for an agent).
