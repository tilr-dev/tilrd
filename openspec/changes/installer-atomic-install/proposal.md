# Proposal: Port the archive installer to the sh repository

## Why

itests UC-004 was red at 13f9522: the installer made `~/.local/bin` before checking the download and installed `tilrd` even when `tilr` failed. `3ba0e93` rewrote the itests copy (`itests/installer/install.sh`) to check the whole release archive in a temporary directory and install both programs or neither; all UC-004 cases pass (commit message, itests/USE_CASES.md group A). The installer users run is the `sh` repository's `install.sh`, served from R2 behind `tilr.sh`; on 2026-10-18 the local copy of that repository (`~/projects/tilr/sh/install.sh`, dated 2026-10-04) is still the old one: it installs `tilr-server` from the loose files and sets up a systemd service. Since `2a8b425` releases are published as the archive only, so that installer finds nothing to install from a new release.

## What Changes

- Replace the `sh` repository's `install.sh` with `itests/installer/install.sh` (archive, temporary directory, all or nothing, no service, hints) and publish it to R2 before the first archive-only release goes to the CDN.

## Capabilities

### Modified Capabilities
- `installation`

No spec delta: the `installation` spec already states the behavior of `itests/installer/install.sh`; `skip_specs: true`.

## Out of scope

- Changes to the installer's behavior beyond the copy in this repository.
- Whether the installer sets up a service (decided: it does not; archived `2026-10-18-installer-service-default`).

## Impact

The `sh` repository and R2 (`https://cdn.tilr.dev/install.sh`). **BREAKING** for anyone running the published installer until the port: it cannot install a release published as the archive only. Nothing crosses a reload.
