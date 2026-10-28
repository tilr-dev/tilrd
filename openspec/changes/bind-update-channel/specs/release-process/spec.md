## MODIFIED Requirements

### Requirement: The CDN layout is one archive per platform
Per platform, `scripts/release.sh` SHALL stage in `dist/cdn` the versioned `tilr-<version>-<platform>.tar.gz`, its `.sha256`, and `releases/<platform>/latest`. Public stable and beta SHALL use `latest` and `beta` respectively, with adjacent `<latest|beta>.manifest.json` authenticating version, channel (`stable` or `beta`), platform, archive name and SHA-256. Both channels SHALL use the same versioned archives and local staging layout.

**Verified by**: itests `UC-001` (`uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root`), whose CDN publishes this layout (`LocalCdn.publish`).

#### Scenario: One platform built
- **WHEN** `scripts/release.sh linux-x86_64` finishes for `0.1.0-26449dd`
- **THEN** its `.sha256` contains `<hex>  <archive>`, the local latest contains `<version>\n`, and the archive holds `tilrd` and `tilr` mode 0755, `LICENSE-APACHE`, `LICENSE-MIT` and `README.md` at its top level
- **AND** `dist/cdn/releases/0.1.0-26449dd/` holds `tilr-0.1.0-26449dd-linux-x86_64.tar.gz` and its `.sha256`, and `releases/linux-x86_64/latest` is `0.1.0-26449dd`
