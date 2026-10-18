# Proposal: Make the repository ready to go public

Блокер: When should tilr-dev/tilrd go public? Options: (a) right after itests merges into main and the first tilrd release; (b) after Cloud production is complete; (c) not planned yet. Recommendation: (a) — the arm64 release build and vulnerability reporting both need it, and gitleaks already guards the history.

Зависит от: readme-code-reconciliation

## Why

Before going public: read through README, TODO and USE_CASES; set topics; set `RELEASE_LINUX_ARM64=true` (the arm64 runner is free only for public repositories); enable private vulnerability reporting (Basic Memory `main/projects/tilr/tilrd-repozitorii-servera-na-git-hub-tilr-dev-tilrd-standart-org-ci-relizy-cdn`). The code review of branch `itests` (79397e2..13f9522, 2026-10-18) adds: itests name private infrastructure (`pb`, `dagger-pb`); README:38-44 links `../apps/mac/PRODUCTION.md` outside the repository.

## What Changes

- Remove or parametrize private infrastructure names in itests; fix out-of-repo links; the checklist above.

## Capabilities

### Modified Capabilities
- `release-process`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Out of scope

- The decision to go public itself (the owner's).

## Impact

Docs, itests configuration, repository settings. No behaviour change.
