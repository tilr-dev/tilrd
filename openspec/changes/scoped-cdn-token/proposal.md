# Proposal: A scoped R2 token for the cdn environment

## Why

The `cdn` GitHub environment uses a broad `R2_TOKEN` (Admin read/write on all R2 of the account); a token scoped to the `tilr-cdn` bucket is an open item (Basic Memory `main/projects/tilr/tilrd-repozitorii-servera-na-git-hub-tilr-dev-tilrd-standart-org-ci-relizy-cdn`).

## What Changes

- Create a token scoped to the bucket with object read/write only; replace the environment secret; drop the old token.

## Capabilities

### Modified Capabilities
- `release-process`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Out of scope

- Nothing beyond What Changes.

## Impact

GitHub environment `cdn` and Cloudflare; no code change unless the variable name changes. No protocol, reload or update impact.
