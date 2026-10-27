# Proposal: Make the repository ready to go public

Deferred: decided when the project is complete.

Зависит от: readme-code-reconciliation

## Decision (2026-10-18)

- Not decided yet: the repository (and the whole project) goes public only when everything is fully working; there is no plan or date. The checklist below is kept for that moment.
- Rejected for now: going public right after the itests merge and the first tilrd release; going public after Cloud production.

## Why

Before going public: read through README, TODO and USE_CASES; set topics; set `RELEASE_LINUX_ARM64=true` (the arm64 runner is free only for public repositories); enable private vulnerability reporting (Basic Memory `main/projects/tilr/tilrd-repozitorii-servera-na-git-hub-tilr-dev-tilrd-standart-org-ci-relizy-cdn`). The code review of branch `itests` (79397e2..13f9522, 2026-10-18) adds: itests name private infrastructure (`pb`, `dagger-pb`); README:38-44 links `../apps/mac/PRODUCTION.md` outside the repository.

## What Changes

- Remove or parametrize private infrastructure names in itests; fix out-of-repo links; the checklist above.

## Capabilities

### Modified Capabilities
- `release-process`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Non-goals

- The decision to go public itself (the owner's, when the project is complete).
- A date or plan for going public.

## Impact

Docs, itests configuration, repository settings. No behaviour change.
