# Release from release branches, integrate on main

## Why
The `dev` branch is gone: `main` is the only integration branch, and a release is cut as `release/X.Y` from a commit of `main` on the owner's instruction (owner's decision of 2026-10-27, Basic Memory `projects/tilr/decisions/`, "Trunk-based в tilr-dev"). The release workflow still runs release-please on every push to `main` and back-merges `main` into a `dev` that no longer exists; the spec-change check compares the base with the branch tip, so a task branch behind `main` looks as if it removed every requirement `main` gained after it forked (owner's comment on the task, 2026-10-27).

## What Changes
- release-please runs only on pushes to `release/**`, with the pushed branch as `target-branch`; a push to `main` releases nothing.
- A push to `release/X.Y` that creates no release tags the next release candidate `vX.Y.Z-rc.N` on the pushed commit and makes a GitHub prerelease with the same archives and checksums as a release; the CDN is not touched.
- Release builds stamp the tag's version into the binaries (`TILR_BUILD_VERSION`), so a candidate's binaries name the candidate.
- The `back-merge` job and its requirement are removed.
- CI also runs on pull requests into and pushes to `release/**`, so the release PR and cherry-picks are checked.
- The spec-change workflow takes the gitpm version: the merge readiness helper `spec_merge.py`, and a comparison of `main` with GitHub's test merge of the pull request. Both shared scripts are downloaded from a fixed gitpm commit, each checked against its pinned digest.
- Specifications, README, AGENTS.md and `openspec/config.yaml` describe the `main` + `release/X.Y` flow.

## Out of scope
- The CDN channel for release candidates and `tilrd update --channel`.
- GitHub settings: rulesets of `release/*`, the `cdn` environment's branch policy (it must allow `release/*` before the first release from a release branch), merge queue.
- Moving the shared scripts' pin to a newer gitpm commit.

## Impact
- `.github/workflows/release.yml`, `ci.yml`, `spec-change.yml`; `scripts/release.sh`, `scripts/linux.sh`; `crates/tilrd/tests/release.rs`; README, AGENTS.md, USE_CASES.md, `openspec/config.yaml`.
- No protocol, reload or CDN layout change; nothing to carry over an update. Not breaking for users: the release artifacts and the CDN layout stay as they are.
