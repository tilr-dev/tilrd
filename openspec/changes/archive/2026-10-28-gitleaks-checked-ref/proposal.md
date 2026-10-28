# Scan only the checked commit's history for secrets in CI

## Why
The gitleaks job checks out with full history, which fetches every branch of the repository, and `gitleaks git` without log options scans all of them. A finding on an unmerged pull request branch then turns CI on `main` red even though `main` itself is clean, and every open pull request and merge queue group fails the same way.

## What Changes
- Limit the CI gitleaks scan to the history of the checked-out commit (`--log-opts=HEAD`) on pull requests, merge queue groups and pushes.
- Use the same command in the developer instructions.
- Extend the static CI test to require the limited scan.

## Impact
- `.github/workflows/ci.yml` (secrets job), `AGENTS.md`, `crates/tilrd/tests/release.rs`.
