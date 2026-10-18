## MODIFIED Requirements

### Requirement: Manual install from the CDN
A manual install SHALL need only the CDN layout: read `releases/<platform>/latest`, download `releases/<version>/tilr-<version>-<platform>.tar.gz`, unpack `tilrd` and `tilr` from it into `~/.local/bin`, and run `tilr`.

**Verified by**: itests `UC-007` (`uc_007.py::a_manual_install_from_the_cdn_layout_runs`, planned in change test-coverage-gaps; id assigned at apply if taken).

#### Scenario: By hand
- **WHEN** a user fetches `latest` for `linux-aarch64` and unpacks `tilrd` and `tilr` of that version's archive into `~/.local/bin`
- **THEN** `tilr` starts the unpacked `tilrd` and attaches
