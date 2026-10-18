# Proposal: Publish the archive installer with the first release

Зависит от: release-process-tests (its `crates/tilrd/tests/release.rs` harness with the stand-in `wrangler` is what the installer upload test extends)

Блокер: Where does `install.sh` live from now on? The served copy comes from `~/projects/tilr/sh`, a directory that is not a git repository (no `tilr-dev/sh` on GitHub either), while the tested copy is `itests/installer/install.sh` here. Options: (a) keep `~/projects/tilr/sh` as the source and copy by hand before each publish (today's way; drift is what made UC-004 red); (b) make `~/projects/tilr/sh` a repository `tilr-dev/sh` (installer + the `tilr.sh` Worker) and keep `itests/installer/install.sh` a checked copy; (c) this repository owns `install.sh` (`scripts/install.sh`, the one itests run), and the release workflow's `cdn` job uploads it after the release's `latest`; `~/projects/tilr/sh` keeps only the `tilr.sh` Worker. Recommendation: (c): one copy, tested by itests in CI, published by the same job that publishes what it installs, so it can never point at a layout that is not there. The tasks below are written for (c); with (a) or (b) group 1 becomes "copy into the sh source, diff, publish with its `publish.sh`" and the spec delta is dropped.

## Why

- itests UC-004 was red at 13f9522: the installer made `~/.local/bin` before checking the download and installed `tilrd` even when `tilr` failed. `3ba0e93` (2026-10-18) rewrote the itests copy (`itests/installer/install.sh`) to check the whole release archive in a temporary directory and install both programs or neither; UC-001..006 pass (commit message, itests/USE_CASES.md group A).
- Since `2a8b425` a release is published as the archive only (`releases/<version>/tilr-<version>-<platform>.tar.gz`, spec `release-process`).
- Checked on 2026-10-18: `https://tilr.sh` (the `tilr-sh` Worker, which serves `install.sh` from the `tilr-cdn` bucket) serves a placeholder that prints `tilr: no public release yet, see https://tilr.dev` and exits 1; `https://cdn.tilr.dev/releases/linux-x86_64/latest` is 404 (nothing released to the CDN yet).
- `~/projects/tilr/sh/install.sh` (2026-10-04) is the pre-archive installer (`tilr-server` from loose files plus a systemd service); `~/projects/tilr/sh-tilrd-rename/install.sh` is byte-identical to `itests/installer/install.sh` at 1163abd. Neither directory is a git repository; `itests/README.md` calls the itests copy "a copy of the installer of the sh repository (not edited here)".
- README.md:56 tells users to install with `curl -fsSL https://tilr.sh | sh`, so the first release is not usable from the README until the archive installer is served.

## What Changes

- `scripts/install.sh` becomes the installer (moved from `itests/installer/install.sh`, unchanged); itests mount it from there (`itests/src/tilr_itests/main.py`, `itests/tt`), README and itests/README say so.
- `scripts/release.sh --publish` / `--upload-only` upload `install.sh` (`text/x-shellscript; charset=utf-8`, `public, max-age=300`) after every platform's `latest`, so the served installer never names a layout the CDN does not have.
- The first release that reaches the CDN replaces the placeholder at `https://tilr.sh`.
- Not breaking: nothing is served today but the placeholder; the CDN layout does not change.

## Capabilities

### Modified Capabilities
- `release-process`: see `specs/release-process/spec.md`.

## Non-goals

- Any change to what the installer does (the `installation` spec already states the behavior of `itests/installer/install.sh`).
- The `tilr.sh` Worker (`~/projects/tilr/sh/src/index.js`) and its domain.
- A service set up by the installer (decided: none; archived `2026-10-18-installer-service-default`).

## Impact

`itests/installer/install.sh` → `scripts/install.sh`, `itests/src/tilr_itests/main.py`, `itests/tt/__init__.py`, `itests/README.md`, `scripts/release.sh`, README "Install", the `release-process` spec. Nothing crosses a reload; an update from the previous release is unaffected (`tilrd update` does not read `install.sh`). Gate: lands before the first release is merged to `main`, so that release's `cdn` job publishes the installer.
