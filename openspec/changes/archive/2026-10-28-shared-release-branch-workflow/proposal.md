## Why
Release orchestration is duplicated in the callers. The shared release-branch workflow is available on the shared repository main branch; gitpm also needs release candidates.

## What Changes
- Delegate branch validation, release-please and candidate selection to the shared release-branch workflow at an immutable main commit.
- Keep artifact builds, uploads, crates.io publication and CDN publication in the calling repository.
- Preserve release-line version selection and stable publication policy.

## Impact
The release workflow, caller integration coverage and release-process specification in this repository.
