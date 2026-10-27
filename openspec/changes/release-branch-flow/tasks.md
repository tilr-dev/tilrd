## 1. Specification
- [x] Add the release branch flow, release candidates and spec merge readiness diagnostics; modify CI triggers, release-please branch and the `cdn` environment's branch; rename spec-change authorization to `main`; remove the back-merge requirement (`openspec validate release-branch-flow --strict`).
- [x] State in «A release is built from a clean commit» and «Linux binaries are built in a Debian 12 container» that `release.sh` takes `TILR_BUILD_VERSION` and passes it into the build (`release::a_release_is_named_after_the_build_version_when_given`, `release::the_linux_build_runs_in_the_pinned_debian_12_image`).

## 2. Implementation
- [x] Run release-please only on pushes to `release/**` with the pushed branch as `target-branch`; drop the `back-merge` job (`release::releases_run_only_on_release_branches`).
- [x] Tag the next release candidate and create its prerelease when a push creates no release (`release::a_push_to_a_release_branch_tags_the_next_release_candidate`).
- [x] Build and attach candidates like releases, with the tag's version in the binaries, and keep the `cdn` job to releases (`release::a_release_candidate_is_built_like_a_release_and_skips_the_cdn`, `release::a_release_is_named_after_the_build_version_when_given`).
- [x] Run CI on pull requests into and pushes to `release/**` (`release::ci_runs_every_gate_of_agents_md`).
- [x] Bring `spec-change.yml` to the gitpm version, both shared scripts from a fixed gitpm commit (`release::the_spec_change_check_compares_main_with_the_test_merge`).
- [x] README, AGENTS.md, USE_CASES.md and `openspec/config.yaml` describe `main` and `release/X.Y`.

## 3. Validation
- [x] `cargo test -p tilrd --test release --features tilrd/failpoints --locked` passes; `actionlint` passes on the workflows.
- [x] `git grep -w dev` leaves only `/dev/null`, the `dev` profile and dependencies, the `-dev` version suffix, history and the spec text this change replaces on archive.
