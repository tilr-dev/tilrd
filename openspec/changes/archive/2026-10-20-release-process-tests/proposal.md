# Proposal: Tests for the release scripts and workflows

Implemented as part of TILR-77.

## Why

Split out of `test-coverage-gaps` (2026-10-18): the `release-process` spec lists 13 of its 16 requirements without a test, and its Notes say "The release scripts and workflows have no automated tests; the CDN layout is exercised only as the itests' `LocalCdn.publish` reproduces it". Checked at 1163abd: `scripts/release.sh` (refusals at :51-63 and :66-69, upload order at :113-125, `--upload-only` checks at :97-108), `scripts/linux-build.sh`, `scripts/install-tools.sh`, `.github/workflows/{ci,release}.yml` and `release-please-config.json` are run by no test in `crates/*/tests/` or `itests/`. The first release is the first time most of this runs; a mistake there reaches the CDN.

## What Changes

- A cargo integration test file `crates/tilrd/tests/release.rs` (runs in CI on Linux and macOS with the rest):
  - Script tests run a copy of `scripts/release.sh` in a temporary git repository with a stand-in `scripts/linux-build.sh` (writes `dist/linux-<arch>/{tilrd,tilr}` scripts that print a version) and a stand-in `wrangler` on `PATH` that logs its arguments: dirty tree, version naming, unknown platform, per-platform `latest`, account id required before anything, upload order (archive, `.sha256`, then `latest`), `--upload-only` checks and leaving other platforms' `latest`.
  - A layout test points `tilrd update`'s reading of the CDN (`crates/tilrd/src/update.rs`, as `crates/tilrd/tests/update.rs` already drives it against a directory served on a loopback port) at the `dist/cdn` the script laid out.
  - Static tests read the workflow files, `scripts/linux.Dockerfile` and `scripts/install-tools.sh` as text and check what the requirements name (the gates of AGENTS.md, the smoke test, pinned checksums, the `cdn` job's `needs` and environment, the targets). They catch an edit that drops a gate; they do not run GitHub Actions.
  - Two macOS SDK requirements added to `release-process` after this change was drafted are covered here as well: the tracked Cargo defaults and workflow selection are checked statically, and the deterministic mocked SDK installer test is run from cargo. This keeps the capability's coverage complete.
- Each `release-process` requirement's **Verified by** names its test (spec delta).

## Capabilities

### Modified Capabilities
- `release-process`: **Verified by** lines only (see `specs/release-process/spec.md`); requirement text unchanged.

## Non-goals

- Running Docker builds, GitHub Actions or wrangler for real in tests.
- Changing the scripts or workflows (a test that finds a mismatch stays red and the fix gets its own change).
- The installer (`installer-atomic-install`, in the sh repository).

## Impact

`crates/tilrd/tests/release.rs` (new), USE_CASES.md, the `release-process` spec's **Verified by** lines and Notes. No behavior, protocol, reload or update impact. Lands before the first release (release gate in `openspec/ROADMAP.md`). If `scoped-cdn-token` changes `scripts/release.sh`'s upload, the stand-in `wrangler` follows it; the MODIFIED copies are refreshed from the current spec before archiving (task 3.2).
