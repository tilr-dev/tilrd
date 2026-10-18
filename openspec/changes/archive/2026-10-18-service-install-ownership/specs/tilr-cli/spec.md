## ADDED Requirements

### Requirement: Service install leaves a unit or plist it did not write
`tilr service install` MUST NOT replace a `tilr.service` (Linux, systemd) or `sh.tilr.tilrd.plist` (macOS) without the marker `tilr service`: it SHALL exit non-zero with `<path> was not written by `tilr service install`: move it away to install`, leave the file as it is and tell the service manager nothing (only `systemctl --user show-environment` runs, to find the manager).

**Verified by**: unit test `service::tests::a_file_without_the_mark_is_not_replaced` (crates/tilr/src/service.rs), itests `UC-020` (`uc_020.py::install_leaves_a_unit_it_did_not_write`).

#### Scenario: A unit it did not write
- **WHEN** `~/.config/systemd/user/tilr.service` exists without `tilr service` in it and the user runs `tilr service install`
- **THEN** install exits non-zero, stderr names the file and says it was not written by `tilr service install`, the file is unchanged, the only call made is `systemctl --user show-environment`, and no crontab is written

#### Scenario: A plist it did not write
- **WHEN** `~/Library/LaunchAgents/sh.tilr.tilrd.plist` exists without `tilr service` in it
- **THEN** install exits non-zero naming it and the file is unchanged

### Requirement: A missing crontab is recognized whatever the cron
`tilr service install` and `uninstall` SHALL run `crontab` with `LC_ALL=C` and take a `crontab -l` that fails saying `no crontab for` (cronie, Vixie cron) or `No such file or directory` (busybox) as an empty crontab; any other failure SHALL still stop install.

**Verified by**: unit test `service::tests::a_missing_crontab_is_recognized_from_cron_and_busybox` (crates/tilr/src/service.rs).

#### Scenario: busybox crontab without a crontab
- **WHEN** `crontab -l` exits 1 saying `crontab: can't open '/var/spool/cron/crontabs/u': No such file or directory`
- **THEN** install takes the crontab as empty and writes its line
