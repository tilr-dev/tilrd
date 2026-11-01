# Two-tier CI: a quick tier on pull requests, the full one in the merge queue

## Why
Every change ran the whole suite four times (locally, on the pull request, in the merge queue and on the push to `main`), and the pull request waited for the macOS job, which also downloaded its build tools on every run. Pull request feedback took about three minutes.

## What Changes
- Pull requests into `main` or `release/**` run the quick tier: the same jobs and check names, the tests on Linux only with the nextest profile `quick` (all but the slow tests); the `test (macos-15)` leg skips its steps and reports success.
- Merge groups, pushes to `release/**`, a nightly run on `main` and a manual run run the full tier: every test on Linux and macOS 15 with failpoints.
- A push to `main` no longer runs CI: what lands there passed its merge group.
- The output of `scripts/install-tools.sh` is cached, keyed by the script; a warm run restores it instead of downloading.
- The nightly run also runs the black-box tests in `itests/` on a local Dagger engine.

## Impact
- Affected specs: release-process
- Affected code: `.github/workflows/ci.yml`, `.config/nextest.toml`, static CI tests, AGENTS.md and README
