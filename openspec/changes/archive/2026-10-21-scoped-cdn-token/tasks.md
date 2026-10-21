# Tasks

## 1. Scoped S3 publishing

- [x] 1.1 Verify derived credentials: PUT and DELETE `tilr-cdn/test/scoped-token` succeed; S3 ListBuckets returns 403 AccessDenied.
- [x] 1.2 Install `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` in environment `cdn`, remove `CLOUDFLARE_API_TOKEN`, and verify secret timestamps.
- [x] 1.3 Replace Wrangler uploads with AWS CLI and update workflow, comments and integration tests while preserving metadata, order and failure behavior.
- [x] 1.4 First production release verification is tracked separately.
- [x] 1.5 Close the task note's open credential item.

## 2. Validation

- [x] 2.1 Run release integration tests and required repository checks.
- [x] 2.2 Confirm `./itests/dt coverage` exits 0.
- [x] 2.3 Review and archive this change.
