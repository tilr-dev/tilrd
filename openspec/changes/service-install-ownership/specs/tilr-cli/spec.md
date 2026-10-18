## MODIFIED Requirements

### Requirement: Service install with systemd
On Linux, when `systemctl --user show-environment` succeeds and lingering is on (`loginctl show-user USER --property=Linger --value` says `yes`) or `loginctl enable-linger USER` succeeds, `tilr service install` SHALL write the user unit `tilr.service` in `$XDG_CONFIG_HOME/systemd/user` (else `~/.config/systemd/user`), run `systemctl --user daemon-reload` and `systemctl --user enable --now tilr.service` (without `--now` when a tilrd already serves the socket), say which way was chosen and exit 0. A `tilr.service` there without the marker `tilr service` MUST NOT be replaced: install SHALL exit non-zero naming it, and run no `systemctl`.

**Verified by**: unit test `service::tests::the_unit_runs_tilrd_from_a_login_shell_and_tells_systemd_when_ready` (crates/tilr/src/service.rs), itests `UC-020` (`uc_020.py::service_install_writes_a_notify_user_unit_and_enables_it`, `uc_020.py::with_lingering_off_install_turns_it_on_and_notes_it`, `uc_020.py::a_running_tilrd_is_left_alone`, `uc_020.py::install_leaves_a_unit_it_did_not_write`).

#### Scenario: Unit content
- **WHEN** install writes the unit for tilrd at `/home/u/.local/bin/tilrd`
- **THEN** the unit starts with `# Written by `tilr service install`; `tilr service uninstall` removes it.` and has `Type=notify`, `ExecStart=/bin/sh -lc 'exec "/home/u/.local/bin/tilrd"'`, `Restart=on-failure`, `RestartSec=2` and `WantedBy=default.target`
- **AND** it is written aside and renamed into place, mode 0644

#### Scenario: Install with lingering off
- **WHEN** lingering is off and `loginctl enable-linger` succeeds
- **THEN** the unit ends with `# tilr service: lingering turned on by install` and `systemctl --user enable --now tilr.service` is run
- **AND** a later install keeps that note

#### Scenario: A tilrd already runs
- **WHEN** a tilrd started by `tilr` serves the socket
- **THEN** the unit is enabled without `--now` and the running tilrd keeps its pid and panes

#### Scenario: A unit it did not write
- **WHEN** `~/.config/systemd/user/tilr.service` exists without `tilr service` in it and the user runs `tilr service install`
- **THEN** install exits non-zero, stderr names the file and says it was not written by `tilr service install`, the file is unchanged and `systemctl --user enable` was not run

### Requirement: Service install with cron
On Linux without a systemd user manager, or when lingering can be neither found on nor turned on, `tilr service install` SHALL say why and put into the user's crontab, keeping every other line and replacing an earlier line of its own, `@reboot /bin/sh -lc 'mkdir -p -m 700 "<dir>" && exec "<tilrd>" --socket "<socket>" >>"<dir>/tilrd.log" 2>&1' # tilr service` (`<dir>` the socket's), print it, and start tilrd now when none runs (`tilrd started`). It SHALL run `crontab` with `LC_ALL=C` and take a `crontab -l` that fails saying `no crontab for` or `No such file or directory` as an empty crontab.

**Verified by**: unit tests `service::tests::the_cron_line_is_recognized_as_ours_and_others_are_not`, `service::tests::a_missing_crontab_is_recognized_from_cron_and_busybox`, itests `UC-021` (`uc_021.py::without_lingering_a_crontab_reboot_line_is_written`, `uc_021.py::without_systemd_a_crontab_reboot_line_is_written_next_to_the_users_own`, `uc_021.py::with_neither_systemd_nor_crontab_install_fails_and_writes_nothing`).

#### Scenario: Next to the user's own line
- **WHEN** the user's crontab has a line of their own and there is no systemd, and install runs twice
- **THEN** the crontab holds the user's line and exactly one `# tilr service` line

#### Scenario: Socket under XDG_RUNTIME_DIR
- **WHEN** the socket is under `$XDG_RUNTIME_DIR`
- **THEN** install also warns that this directory is made at login and gone at boot, and to set `TILR_RUNTIME_DIR` and install again

#### Scenario: Neither systemd nor crontab
- **WHEN** neither `systemctl` nor `crontab` works
- **THEN** install exits non-zero with `no way to start tilrd at boot here: no systemd user service with lingering, and no crontab (…)`, writes nothing in the home and starts nothing

#### Scenario: busybox crontab without a crontab
- **WHEN** `crontab -l` exits 1 saying `crontab: can't open '/var/spool/cron/crontabs/u': No such file or directory`
- **THEN** install takes the crontab as empty and writes its line

### Requirement: Service install on macOS
On macOS, `tilr service install` SHALL write `~/Library/LaunchAgents/sh.tilr.tilrd.plist` and, unless a tilrd already serves the socket, run `launchctl bootout gui/<uid>/sh.tilr.tilrd` and then `launchctl bootstrap gui/<uid> <plist>`, failing on a bootstrap error. A plist there without the marker `tilr service` MUST NOT be replaced: install SHALL exit non-zero naming it, and run no `launchctl`.

**Verified by**: unit tests `service::tests::the_plist_names_the_socket_and_keeps_tilrd_alive_on_failure`, `service::tests::a_file_without_the_mark_is_not_replaced`.

#### Scenario: Plist content
- **WHEN** install runs with socket `/tmp/s & t/tilrd.sock`
- **THEN** the plist has label `sh.tilr.tilrd`, `ProgramArguments` `/bin/sh -lc 'exec "<tilrd>" --socket "/tmp/s &amp; t/tilrd.sock"'`, `RunAtLoad` true, `KeepAlive` with `SuccessfulExit` false, and stdout and stderr in `~/Library/Logs/tilrd.log`

#### Scenario: A plist it did not write
- **WHEN** `~/Library/LaunchAgents/sh.tilr.tilrd.plist` exists without `tilr service` in it
- **THEN** install exits non-zero naming it and the file is unchanged
