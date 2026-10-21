# Proposal: Scope CDN credentials to the release bucket

## Why

The CDN uploader needs object access to `tilr-cdn`, while its existing credentials allow administration of all R2 buckets. Bucket-scoped object permissions require the R2 S3 API rather than Wrangler's REST upload API.

## What Changes

- Upload through AWS CLI to the account-specific R2 S3 endpoint, retaining object keys, metadata and publication order.
- Derive S3 credentials from the existing `R2_CDN_TOKEN` in Infisical `/cloudflare`, environment `prod`: access key is the token ID and secret key is SHA-256 of the token value.
- Store `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` in GitHub environment `cdn`; remove its `CLOUDFLARE_API_TOKEN`. Keep the `CLOUDFLARE_ACCOUNT_ID` variable.
- Verify object PUT/DELETE succeed and ListBuckets is denied with 403. Do not revoke the account's original `R2_TOKEN`, which other tools may use.

## Capabilities

### Modified Capabilities
- `release-process`: S3 uploads and credentials, including the installer publication tests.

## Non-goals

- Tokens of other repositories or the manually deployed installer redirect Worker.
- Changing release layout, public URLs, cache policy or publication order.
- Exercising the first production release, which is tracked separately.

## Impact

`scripts/release.sh`, its integration tests, `.github/workflows/release.yml`, and credentials of GitHub environment `cdn`. AWS CLI replaces the npm Wrangler dependency for this job.
