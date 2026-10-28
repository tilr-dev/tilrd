## Why
Release orchestration is duplicated in gitpm and tilrd; only tilrd currently tags release candidates. Both repositories now use the release-line runner introduced in their release workflows.

## What Changes
- Introduce a shared release-branch workflow in tilr-dev/.github and call it from gitpm and tilrd at an immutable commit.
- Keep one release-line runner and regression suite, validate release branch names, and expose release/candidate metadata.
- Keep artifact builds, uploads, crates.io publication and CDN publication in their calling repositories.

## Non-goals
- Moving artifact builds, signing, notarization or CDN upload into the shared repository.
- Changing release-line version selection or stable publication policy.

## Impact
Callers: gitpm and tilrd. This introduces a new workflow contract; no existing shared workflow contract is replaced.
