# tilrd-tests — сценарии (черновик)

Чёрный ящик: настоящий бинарь tilrd в Linux-контейнере, клиент (`tilr-driver`, для
группы B — сам `tilr`) в соседнем, сеть между ними портит прокси или netem. Гоняется на Dagger Engine в pb
по образцу `pm-dagger`: каждый кейс — свой набор контейнеров, всё параллельно,
отчёт PASS / FAIL / BLOCKED.

Протокольные сценарии одной машины уже покрыты cargo-тестами
(`tilrd/USE_CASES.md`). Здесь их не повторяем, а гоняем как группу R.

Решения (2026-10-16):
- Драйвер — `crates/tilr-driver` в репо tilrd (publish = false): JSON-команды на stdin,
  события на stdout, одно соединение на весь сценарий, токен, reconnect, файлы, потоки.
- Всё в репо tilrd: `itests/` (Dagger-модуль, сценарии, этот каталог).
- Прокси слоя 1 — свой, Python asyncio, понимает WS-фреймы, управляется из кейса.
- Сервис опционален; systemd проверяем только по файлам юнитов.
- Ввод в момент обрыва может потеряться, но не дублироваться.
- Платформы: Ubuntu LTS, Debian, Fedora, Alpine (musl), aarch64.
- Два бэкенда одних и тех же сценариев: `dagger` (контейнеры на pb, параллельно)
  и `local` (процессы на маке, настоящий macOS). Кейс, которому нужен контейнер
  (второй IP, другой uid, tmpfs, netem), на `local` — BLOCKED.

Нереализованное пишется сразу и остаётся красным FAIL, пока фичи нет.
Пометки: **[нет фичи]** — тест-спецификация.

## R. Регрессия — существующие cargo-тесты

- R-1. Все `crates/tilrd/tests/*.rs` с `--features failpoints`, один тест на контейнер.
- R-2. clippy, fmt.

## A. Установка и первый запуск (без сервиса)

Реализовано (2026-10-17): все UC-001…006. Инсталлер — `scripts/install.sh`
(его отдаёт `tilr.sh`; в контейнере `/opt/tt/install.sh`), его отдаёт `t.cdn()`
вместе с релизом из этого дерева (один архив `tilr-<version>-<platform>.tar.gz` с `tilrd`
и `tilr` и его `.sha256`, `LocalCdn.publish`); ставит пользователь, созданный кейсом
(`t.user()`: `useradd -m`, чистое окружение логина без `XDG_*`, без sudo, без systemd).
Инсталлер качает архив во временный каталог (`mktemp -d`), проверяет сумму, распаковывает,
запускает `--version` обоих и только потом создаёт `~/.local/bin` и кладёт оба; сервис не
ставит, в конце печатает подсказки (`tilr`, `tilr service install`, `tilr token`,
`tilr cloud login`).

- UC-001. `curl tilr.sh | sh` без root: бинарь в `~/.local/bin`, контрольная сумма сошлась, версия печатается. → `uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root` (оба бинаря 0755 пользователя, sha256 = файлы архива, запрошены архив и его `.sha256`, в доме новое только `~/.local/bin/{tilrd,tilr}`, в `/tmp` ничего не осталось, подсказки напечатаны, `/usr/local/bin` и `/etc` не тронуты; `tilr` поднимает установленный tilrd от пользователя)
- UC-002. `~/.local/bin` нет в PATH — инсталлер говорит, что добавить; ничего не правит молча. → `uc_002.py::without_local_bin_on_path_the_installer_says_what_to_add_and_edits_nothing` (готовая строка `export PATH="$HOME/.local/bin:$PATH"` для `~/.profile`, и она работает; в доме новые только `~/.local/bin/{tilrd,tilr}`)
- UC-003. Повторная установка поверх: идемпотентно, работающий tilrd не убит (или перезагружается без потери шеллов). → `uc_003.py::installing_the_same_release_again_changes_nothing` (та же версия: ничего не скачано, те же inode и mtime, нет reload), `::installing_a_new_release_reloads_the_running_server_and_keeps_its_shells` (тот же pid, панели, `$KEPT`, `$$` и job; драйвер по TCP 7890 с токеном)
- UC-004. Битая контрольная сумма / оборванная загрузка / битый архив / в архиве нет бинаря или бинарь не запускается / нет релиза под платформу — ничего не установлено, мусора нет (ни в доме, ни в `/tmp`), код выхода не 0. → `uc_004.py::a_bad_checksum_installs_nothing`, `::a_download_cut_short_installs_nothing`, `::an_archive_cut_short_installs_nothing`, `::an_archive_without_tilr_installs_nothing`, `::a_program_that_does_not_run_installs_nothing`, `::no_release_for_the_platform_installs_nothing`, `::a_latest_version_without_its_archive_installs_nothing`, `::a_broken_update_leaves_the_installation_and_its_server_alone`
- UC-005. Чистая система без `$XDG_RUNTIME_DIR`, без `$HOME/.config` — пути создаются, права 0700/0600. → `uc_005.py::on_a_clean_system_tilr_makes_its_directories_private` (`/tmp/tilr-<uid>` и всё в нём, `~/.local/state/tilr` 0700, токен и server-id 0600)
- UC-006. Запуск не от root, без sudo, в контейнере без systemd — всё работает. → `uc_006.py::without_root_sudo_or_systemd_everything_works` (sudo — ловушка, не вызван)
- UC-007. Ручная установка из CDN: прочитать `latest`, скачать архив, распаковать `tilrd` и `tilr` в `~/.local/bin` и запустить установленный `tilr`. → `uc_007.py::a_manual_install_from_the_cdn_layout_runs` (отдельный пользователь, запросы к `LocalCdn`, бинарники совпадают с архивом, клиент подключается к установленному серверу)

## B. Модель как у tmux — жизнь демона

Реализовано (2026-10-17): все UC-010…017, `scenarios/uc_0NN.py::<кейс>`. Клиент — `tilr`
(`crates/tilr`) рядом с `tilrd` в `/usr/local/bin`; интерактивный `tilr` гоняется в pty
(`t.session("tilr")`, окно 100×30), приглашение — `Session.prompt()`; пользователь без
демона — `t.user_env()` (свой `XDG_RUNTIME_DIR` 0700). Сокет — `$XDG_RUNTIME_DIR/tilr/tilrd.sock`.

- UC-010. `tilr` без работающего tilrd поднимает его сам (setsid, отвязан от терминала) и подключается. → `uc_010.py::tilr_without_a_running_daemon_starts_tilrd_detached_and_connects`
- UC-011. Второй `tilr` подключается к тому же tilrd, второй демон не появляется. → `uc_011.py::a_second_tilr_attaches_to_the_same_daemon_and_pane` (та же панель, ввод одного виден другому)
- UC-012. Два `tilr` стартуют одновременно на пустой машине — демон ровно один. → `uc_012.py::tilr_started_eight_times_at_once_starts_one_daemon, ::two_terminals_running_tilr_at_once_share_one_daemon` (второй tilrd даже не запускается: lock берёт `tilr` и передаёт демону)
- UC-013. «SSH оборвался»: сессия/терминал клиента убиты (SIGHUP группе) — tilrd и шеллы живы, новый `tilr` видит панели как были. → `uc_013.py::tilr_in_an_ssh_session_leaves_tilrd_and_its_shell_when_the_session_drops` (+ `::tilrd_started_from_an_ssh_login_shell_survives_the_session_dropping`, `::tilrd_started_with_setsid_keeps_its_panes_when_the_ssh_session_drops`: сервер, запущенный руками)
- UC-014. Протухший сокет (tilrd убит SIGKILL) — следующий `tilr` убирает его и поднимает новый демон. → `uc_014.py::after_a_sigkill_the_next_tilr_replaces_the_socket_and_starts_a_new_daemon` (подключённый `tilr` пишет `[tilrd stopped]`, `tilr ls` ничего не поднимает)
- UC-015. Чужой процесс слушает сокет / файл не сокет — внятная ошибка, ничего не удалено. → `uc_015.py::another_programs_listener_at_the_socket_is_left_alone, ::a_file_that_is_not_a_socket_is_left_alone` (и `tilr`, и `tilrd`)
- UC-016. tilrd запущен вручную в foreground (как под сервисом) — лог в stderr, `tilr` к нему цепляется. → `uc_016.py::tilr_attaches_to_a_tilrd_running_in_the_foreground`
- UC-017. `tilr kill-server` / остановка — шеллы завершены, сокеты и state убраны, сирот нет (`ps`). → `uc_017.py::kill_server_leaves_no_process_and_no_socket` (шелл, игнорирующий SIGHUP, — UC-086)

## C. Сервис — опционально, только файлы

Реализовано (2026-10-17): `tilr service install|uninstall` (README «Starting at boot»).
systemd нет в контейнере: `systemctl`, `loginctl`, `crontab` — скрипты, которые пишут свои
аргументы (`tt.FakeServiceTools`); юнит и crontab проверяются как файлы, `Type=notify` —
запуском `ExecStart` юнита с `$NOTIFY_SOCKET`.

- UC-020. `tilr service install`: user unit записан верно (`ExecStart`, `Type=notify`, Restart), `enable` вызван. → `uc_020.py::service_install_writes_a_notify_user_unit_and_enables_it` (tilrd шлёт `READY=1`; в его панелях нет `NOTIFY_SOCKET`), `::with_lingering_off_install_turns_it_on_and_notes_it`, `::a_running_tilrd_is_left_alone` (`enable` без `--now`), `::install_leaves_a_unit_it_did_not_write` (чужой `tilr.service` без метки `tilr service`: install падает, называя файл, файл цел, systemd ничего не сказано)
- UC-021. linger недоступен → crontab `@reboot` записан, вывод говорит, какой способ выбран. → `uc_021.py::without_lingering_a_crontab_reboot_line_is_written`, `::without_systemd_a_crontab_reboot_line_is_written_next_to_the_users_own` (строка пользователя цела, повторный install не дублирует), `::with_neither_systemd_nor_crontab_install_fails_and_writes_nothing`; строка `@reboot`, запущенная как cron, поднимает tilrd на сокете пользователя
- UC-022. `tilr service uninstall` убирает всё, что поставил install, и только это. → `uc_022.py::uninstall_removes_the_unit_it_wrote_and_nothing_else` (disable, linger выключен, другой юнит цел, сервер и панели живы), `::uninstall_leaves_lingering_it_did_not_turn_on`, `::uninstall_removes_its_crontab_line_and_keeps_the_users`, `::uninstall_removes_the_crontab_it_made`, `::uninstall_leaves_what_it_did_not_write` (написанный руками юнит не трогает)
- UC-023. Без `service install` нигде нет автозапуска (явность, не руткит). → `uc_023.py::nothing_starts_tilrd_at_boot_unless_asked` (systemd user manager есть, linger выключен; установка и все команды `tilr`: ни юнита, ни crontab, ни linger, ни autostart, профили и `/etc` не тронуты; инсталлер не спрашивает systemctl, loginctl и crontab вообще)

## D. Сокеты — unix и TCP

Реализовано (2026-10-17): все UC-030…039. TCP по умолчанию — `127.0.0.1:7890` и у
голого `tilrd`, и у поднятого `tilr` (порт занят — такой работает без TCP).

- UC-030. Слушает только 127.0.0.1 по умолчанию; с другого контейнера порт закрыт. → `uc_030.py::a_tilrd_tilr_starts_listens_on_loopback_only`, `::a_bare_tilrd_listens_on_loopback_only` (хост `peer`: refused)
- UC-031. `--listen 0.0.0.0` / `[::]` — доступен снаружи, токен обязателен. → `uc_031.py::listening_on_every_ipv4_address_asks_for_the_token`, `::listening_on_every_address_takes_ipv4_too_and_asks_for_the_token` (без токена и с чужим — отказ, с токеном — шелл)
- UC-032. IPv6 loopback и dual-stack. → `uc_032.py::ipv6_loopback_serves_ipv6_only`, `::dual_stack_serves_both_loopbacks` (без IPv6 на lo — BLOCKED)
- UC-033. Порт занят — ошибка с понятным текстом, код выхода не 0. → `uc_033.py::a_taken_port_is_a_clear_error` (exit 1 за миллисекунды, «… is taken …; pick another port with --listen, or none with --no-listen», сокета нет), `::a_bare_tilrd_next_to_a_running_one_says_the_default_port_is_taken`, `::a_tilrd_tilr_starts_goes_on_without_the_taken_port`
- UC-034. Клиентский unix-сокет демона (B): права 0600, каталог 0700; другой uid подключиться не может. → `uc_034.py::another_user_cannot_use_the_socket` (uid 65534 через `setpriv`: сначала не пускают права, с открытыми руками правами — сервер по uid собеседника)
- UC-035. Unix-сокет `$TILR_SOCK` панели: другой uid не может слать `tilr edit`. → `uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane` (сокет 0600 в 0700; с открытыми правами сервер сверяет uid; так же сокет агента)
- UC-036. Сокеты агента `agent/<pid>/<pane>.sock` после SIGKILL сервера — следующий старт убирает вместе с каталогом `agent/<pid>/`. → `uc_036.py::agent_sockets_left_by_a_killed_server_are_removed_at_the_next_start` (каталог живого pid остаётся, даже с сокетом, который ещё не слушает)
- UC-037. Длинный путь к сокету (>108 байт `sun_path`) — внятная ошибка или обход. → `uc_037.py::a_socket_path_longer_than_sun_path_is_a_clear_error` (ошибка с подсказкой `TILR_SOCKET`; с ним — работает; голый `tilrd` с TCP — предупреждение)
- UC-038. Много соединений сразу (1000 TCP), лимит соединений, медленный handshake (slowloris) — сервер жив, остальные клиенты работают. → `uc_038.py::a_thousand_silent_connections_and_a_slowloris_leave_the_server_working` (лимита числа соединений нет: незавершённый за 10 с handshake закрывается)
- UC-039. Полуоткрытый TCP от клиента (connect и тишина) — отваливается по таймауту handshake; открывший WebSocket и не приславший Hello — через 10 с после handshake, независимо от `--client-timeout`. → `uc_039.py::a_silent_connection_is_closed_by_the_handshake_timeout` (~10 с), `::a_websocket_that_never_says_hello_is_dropped_by_the_client_timeout`, `::a_websocket_that_never_says_hello_is_closed_after_10_s` (`--client-timeout 0`, close 4002 `timeout`)
- UC-03A. `tilr` не говорит с сокетом чужого пользователя. Другой пользователь заранее создал `/tmp/tilr-<uid>` жертвы (нет `XDG_RUNTIME_DIR`) и слушает в нём фальшивый сокет — `tilr ls` / `attach` / `new -d` отказывают, называя каталог и uid владельца, никто не подключился, tilrd не поднят; чужой сокет по `TILR_SOCKET` / `--socket` / `socket` в `client.toml` — соединение закрыто до handshake («socket … is served by another user (uid N)»), не отправлено ни байта. → `uc_03a.py::a_runtime_directory_another_user_made_in_tmp_is_not_entered`, `::a_socket_another_user_serves_at_a_named_path_is_refused_before_a_byte`, `::tilr_edit_says_nothing_to_a_host_socket_another_user_serves` (чужой сокет в `TILR_SOCK`: `tilr edit`, `tilr open`, `tilrd edit` и `tilr` панели выходят с 1, «is served by another user (uid N)», ни байта)

## E. Плохая сеть и переподключение

Слой 1 — управляемый прокси между клиентом и tilrd (ручки: `freeze[dir]`,
`drop_oneway(dir)`, `delay(ms, jitter)`, `loss(pct)`, `cut_rst`,
`freeze_then_resume(s)`, `cut_after_n_messages(n)`). Слой 2 — netem/iptables на
интерфейсе контейнера клиента.

Во всех: в панели идёт непрерывный счётчик; после восстановления экран без
потерь и дублей; меряем миллисекунды до обнаружения и до возврата.

Реализовано (2026-10-17): все UC-040…052, `scenarios/uc_0NN.py::<кейс>`; ручки прокси,
хосты, netem/iptables — в README. Пороги H ещё не заданы: метрики пишутся в report.json,
проверяются инварианты и щедрые верхние границы.

- UC-040. Чистый half-open: DROP в обе стороны посреди вывода — обе стороны замечают за N пингов, клиент возвращается в ту же панель. → `uc_040.py::a_frozen_connection_is_dropped_by_both_sides_and_the_client_returns_to_the_same_pane`
- UC-041. Асимметричный обрыв: только сервер→клиент. Отдельно — только клиент→сервер. → `uc_041.py::a_cut_of_server_to_client_only_is_noticed_by_the_client_and_both_sides_let_go, ::a_client_that_still_talks_but_no_longer_hears_is_dropped_by_the_server (сервер считает понги на свои пинги, а не любые сообщения клиента), ::a_cut_of_client_to_server_only_is_noticed_by_the_server_and_by_a_client_that_counts_pongs`
- UC-042. Зомби-сессия: старое соединение висит half-open, клиент пришёл с нового IP — сервер схлопывает старое, «призрачного» зрителя нет: `Hello` с тем же `client_id` вытесняет старое соединение сразу (< 1 с), не по таймауту. → `uc_042.py::a_client_back_from_a_new_ip_leaves_no_ghost_of_its_half_open_connection (iptables), ::a_client_back_from_a_new_ip_leaves_no_ghost_when_its_old_path_froze_in_a_proxy (3 контейнера)`
- UC-043. Lossy-network degradation and recovery: random 30% loss and jitter report latency, stalls and disconnects; after healing, the screen, input and one viewer recover. Random retransmission backoff cannot guarantee echo p90 below 5 s, and uplink silence may expire the server timeout before the client receives its close. Controlled cases prove both states and recovery; the zero/3000/zero ms each-way case checks the echo threshold with complete samples and no disconnects. No quarantine. → `uc_043.py::a_lossy_slow_network_slows_the_session_without_tearing_it_down_or_hanging_it` (netem), `::a_session_through_a_lossy_slow_proxy_keeps_moving_without_a_disconnect` (proxy; legacy name), `::fixed_network_delay_moves_echo_p90_across_five_seconds`, `::fixed_delay_does_not_fail_the_lossy_proxy_recovery_case`, `::uplink_silence_can_drop_a_client_that_still_receives_frames`
- UC-044. Воскрешение: DROP 10 с, клиент уже переподключился, старое соединение ожило — живо ровно одно; переподключение (тот же `client_id`) вытеснило старое сразу, призрака нет и до оживления. → `uc_044.py::a_revived_connection_the_client_closed_ends_at_once_leaving_one_live_connection, ::a_revived_connection_left_open_by_the_client_was_superseded_by_its_reconnect`
- UC-045. RST посреди кадра — клиент переподключается сразу, сервер чистит зрителя. → `uc_045.py::a_reset_in_the_middle_of_a_frame_makes_the_client_reconnect_at_once, ::a_socket_reset_by_the_clients_kernel_is_cleaned_up_by_the_server_at_once (ss -K)`
- UC-046. Обрыв посреди передачи файла (запись кусками) — файл прежний, временного нет; повтор проходит. → `uc_046.py::a_write_cut_by_a_reset_leaves_the_file_as_it_was_and_no_temporary_file, ::a_write_cut_by_a_silent_network_leaves_the_file_as_it_was_and_no_temporary_file`, `::a_finished_write_is_synced_before_it_is_acknowledged` (под `strace`: fsync временного файла → rename → fsync каталога, всё до `FsAck`)
- UC-047. Обрыв посреди потока агента / SOCKS — программа получает конец, следующий поток работает. → `uc_047.py::an_agent_stream_cut_by_a_reset_ends_for_the_program_and_the_next_one_works, ::an_agent_stream_on_a_silent_connection_…, ::a_socks_stream_cut_by_a_reset_…, ::a_socks_stream_on_a_silent_connection_…`
- UC-048. Input admitted to the delivery queue before or during a connection break reaches the pane exactly once, in order, after reconnecting, including when the old connection revives. This covers reset, silent connection and revived-connection cases. → `scenarios/uc_048.py`
- UC-049. Медленный клиент (полоса 10 КБ/с) при большом выводе — сервер его не копит бесконечно: пропускает ему кадры и шлёт последний экран целиком, остальные клиенты не тормозят; клиент, не успевающий и так (1 КБ/с, понги не приходят за таймаут), отключается, и соединение действительно закрывается. → `uc_049.py::a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others, ::a_client_too_slow_even_for_the_latest_screen_is_dropped_and_its_connection_ends`
- UC-050. Тысяча циклов обрыв/возврат — зрителей не копится, память сервера не растёт. → `uc_050.py::a_thousand_resets_leave_one_viewer_and_flat_server_memory_and_descriptors`
- UC-051. netem: 3G-профиль (delay 300, loss 2%, reorder) — интерактив работает, задержка ввода в метриках. → `uc_051.py::a_shell_stays_usable_on_a_3g_network_and_input_latency_is_recorded`
- UC-052. Идентичность клиента: `Hello.client_id` (UUID v4 на экземпляр — окно, вкладка, процесс — один на все его переподключения). Две вкладки с одного хоста (разные `client_id`) живут обе; тот же `client_id` с чужим токеном никого не вытесняет (с токеном — отказ, без токена — другой клиент); тот же `client_id` и токен — старое соединение закрывается сразу кодом 4000 `superseded` и не переподключается, счётчик в `/metrics`. → `uc_052.py::two_tabs_of_one_host_with_client_ids_of_their_own_both_stay_live (хост), ::the_same_client_id_under_another_token_supersedes_nobody, ::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`

## F. Обновление и перезагрузка с живым софтом в панелях

В панелях заранее запущены: шелл с переменной и фоновой задачей, счётчик,
`vim` с несохранённым буфером, `htop`, `less` по большому файлу, `python` REPL,
долгий `sleep`, программа с открытым TCP-соединением наружу.

Реализовано (2026-10-17): все UC-060…069, `scenarios/uc_0NN.py::<кейс>`; живой софт в
панелях — `tt/live.py` (`LivePanes`), эхо-сервис наружу — контейнер `echo`, свой CDN
кейса — `t.cdn()`, маленький диск — `t.tmpfs()`.

- UC-060. `tilr update` с подставного CDN (отдельный контейнер) — версия новая, pid тот же, весь софт жив и откликается, экран тот же. → `uc_060.py::update_from_the_cdn_keeps_every_program_in_the_panes_alive` (+ python REPL, TCP к контейнеру `echo`, режимы экрана, сироты, дескрипторы — ровно столько же, сколько до обновления: за унаследованным потомком сервер следит через pidfd, как tokio за порождённым); `::an_update_is_synced_before_it_reports_success` (под `strace`: оба распакованных файла — fsync до первого rename, каталог — fsync после второго, всё до `updated`)
- UC-061. vim/htop (alt screen, мышь, скрытый курсор) после перезагрузки — экран и режимы те же, `:wq` работает. → `uc_061.py::vim_keeps_its_screen_modes_and_buffer_across_a_reload_and_writes_it, ::htop_keeps_its_modes_across_a_reload_and_answers_keys_and_the_mouse`
- UC-062. Ctrl+C после перезагрузки прерывает `sleep`; Ctrl+Z / `fg`. → `uc_062.py::ctrl_c_after_a_reload_interrupts_a_foreground_sleep, ::a_sleep_running_through_a_reload_is_interrupted_by_ctrl_c, ::ctrl_z_and_fg_after_a_reload_stop_and_resume_a_job`
- UC-063. A client attached during an update reconnects automatically; admitted input is not lost and first arrivals retain their order. Input applied without a received acknowledgement may be replayed after reload discards delivery history. → `uc_063.py::a_client_typing_through_an_update_reconnects_and_nothing_typed_is_lost_or_doubled` (the callable name is retained for existing coverage references)
- UC-064. Обновление при открытой передаче файла / SOCKS / агенте. → `uc_064.py::a_file_write_broken_by_an_update_leaves_the_file_as_it_was_and_the_retry_works, ::a_socks_session_open_during_an_update_ends_for_the_service_and_a_new_one_works, ::an_agent_stream_open_during_an_update_ends_for_the_program_and_the_next_one_works`. Был красным до 2026-10-17 (временный файл прерванной записи оставался навсегда): теперь запись отмечает свой временный файл в `writes/` каталога выполнения, и запустившийся или перезагрузившийся сервер убирает файлы записей, чей процесс умер или был им самим
- UC-065. Кончилось место на диске (маленький tmpfs) посреди скачивания и при записи состояния — ничего не сломано, шеллы целы. → `uc_065.py::an_update_that_runs_out_of_disk_while_downloading_installs_nothing_and_breaks_nothing, ::a_reload_with_no_room_for_its_state_keeps_the_server_and_its_panes, ::a_reload_with_room_for_the_servers_state_but_not_the_panes_keeps_every_pane`. Был красным до 2026-10-17 (недописанный файл состояния оставался): теперь состояние пишется во временный файл рядом, fsync, rename; при ошибке временный удаляется (reload.rs `write_file`)
- UC-066. Откат на старую версию и обратно с открытыми панелями. → `uc_066.py::rolling_back_and_forward_again_keeps_the_panes_and_their_programs` (обновление ставит `tilrd` и `tilr` из архива; после каждого шага у обоих версия шага)
- UC-067. Новая версия с другим форматом состояния — перезагрузки нет, шеллы целы. → `uc_067.py::a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on`
- UC-068. Перезагрузка в момент создания панели. → `uc_068.py::panes_created_while_the_server_reloads_are_whole_or_absent_and_every_reload_happens`
- UC-069. 50 перезагрузок подряд под нагрузкой — сирот и утечек fd нет (дескрипторов ровно столько же, сколько до первой перезагрузки). → `uc_069.py::fifty_reloads_under_load_leave_no_orphans_and_no_descriptor_leaks`
- UC-06A. Новая версия падает при возобновлении (в проверочном прогоне `TILR_RELOAD_CHECK` и в живом resume) — сервер в неё не exec'ается: тот же pid и слушающий сокет, клиент подключён, старая версия в `HelloAck`, весь софт в панелях жив, файлов состояния и проверки не осталось; `SIGUSR1` отказывается так же. → `uc_06a.py::a_version_that_crashes_resuming_is_not_reloaded_into_and_the_shells_live_on` (до проверки возобновления такое обновление убивало сервер и все шеллы)
- UC-06B. Отказ от перезагрузки у сервера, поднятого `tilr` (`--socket`, `--lock-fd`): сокет и его lock остаются, `tilr ls` видит ту же панель, новый `tilr` цепляется к тому же tilrd (тот же шелл, `$KEPT`), второго демона нет. → `uc_06b.py::a_refused_reload_keeps_the_socket_and_tilr_finds_the_same_server`

## G. Передача: файлы, данные по каналу, команды

Реализовано (2026-10-17): все UC-070…07A, `scenarios/uc_0NN.py::<кейс>`; новые команды
драйвера (файлы в фоне, буфер обмена, мышь, режимы агента, локальный SOCKS, черновик
Exec) — в README. Пороги задержки щедрые, как в E: метрики пишутся в report.json.

- UC-070. Файл туда и обратно: 0 байт, 1 байт, ровно лимит, лимит+1, 1 ГБ кусками; хэш совпадает. → `uc_070.py::files_of_zero_one_and_exactly_the_limit_go_both_ways_intact, ::a_file_one_byte_over_the_limit_is_refused_whole_and_read_in_parts, ::a_gigabyte_goes_up_and_comes_back_in_chunks_with_its_hash`
- UC-071. Бинарные имена, пробелы, юникод, симлинки, права сохраняются. → `uc_071.py::odd_and_unicode_names_go_both_ways_and_keep_their_exact_bytes, ::a_name_that_is_not_utf8_is_listed_so_that_it_can_be_read_back, ::symbolic_links_are_listed_followed_and_written_through_but_not_out_of_the_shared_directories, ::permission_bits_are_kept_by_writes_and_told_by_listings`. Был красным до 2026-10-17: теперь в proto у каждого пути есть `bytes`-близнец (`path_bytes`, `FsEntry.name_bytes` и др.), сервер идёт по байтам; драйвер передаёт такие имена в JSON как surrogateescape Python
- UC-072. Параллельные передачи нескольких файлов и терминал одновременно — терминал не тормозит. → `uc_072.py::parallel_transfers_both_ways_leave_the_terminal_responsive_and_the_files_intact, ::parallel_reads_leave_the_terminal_responsive, ::parallel_writes_leave_the_terminal_responsive`. Был красным до 2026-10-17 (эхо до 1,6 с при чтениях, до 5 с в обе стороны): теперь у очереди клиента две полосы, кадры идут раньше FsData/StreamData, а массовых данных «в сети», ещё не прочитанных клиентом, не больше 1 МиБ (по pong'ам). Драйвер собирается release (как настоящий клиент): debug-драйвер читает ~30 МБ/с, и p99 в обе стороны с ним ~350 мс
- UC-073. Данные по каналу (StreamOpen/Data/Close): пропускная способность, порядок байт, большой поток, обратное давление (получатель медленный). → `uc_073.py::a_stream_echoed_by_the_client_comes_back_whole_in_order, ::a_big_stream_to_the_client_arrives_whole_and_in_order, ::a_slow_client_holds_the_program_back_and_is_neither_dropped_nor_buffered_for, ::a_client_running_ahead_of_a_slow_program_is_held_to_the_queue_and_then_loses_the_stream`. Был красным до 2026-10-17 (буфер до 64 МБ): теперь полоса массовых данных клиента не больше 4 МиБ, насос агента ждёт места — программа сдерживается, память сервера ровная
- UC-074. SOCKS: curl к сервису на loopback сервера через клиента; несколько сессий; большой ответ. → `uc_074.py::curl_on_the_clients_machine_reaches_a_service_on_the_servers_loopback, ::twenty_sessions_at_once_each_get_their_own_file, ::a_big_response_through_socks_arrives_whole` (хост `laptop`)
- UC-075. Буфер обмена: картинка байт в байт, лимит, истечение. → `uc_075.py::an_image_put_on_the_clipboard_is_pasted_byte_for_byte, ::the_clipboard_takes_exactly_its_limit_and_refuses_one_byte_more_leaving_it_empty, ::clipboard_content_expires`
- UC-076. Команды из панели к клиенту: `tilr edit` / `tilr open` доходят всем, кто смотрит панель. → `uc_076.py::tilr_edit_and_open_reach_every_client_watching_the_pane_and_no_other (хост), ::tilr_edit_with_nobody_listening_fails_and_says_why`
- UC-07D. Копирование из панели к клиенту: `:w !xclip -selection clipboard -i` в vim, OSC 52 из вывода и `xclip -t image/png -i` картинки доходят клиенту, который смотрит панель и объявил `"clipboard-write"` (`ClipboardWrite`), байт в байт; клиенту без `"clipboard-write"` — ничего. → `uc_07d.py::a_copy_in_vim_reaches_the_clients_clipboard`
- UC-07B. Профиль, задающий `PATH` с нуля, не прячет инструменты панели: в bash, zsh и fish `$TILR_SHIMS` первым в `PATH` (один раз), файлы пользователя выполняются, оболочка — login (`-bash`, `-zsh`, `-fish`), свой `ZDOTDIR` пользователя сохраняется; dash — как раньше, с `$TILR_SHIMS` для своей правки. → `uc_07b.py::the_tools_stay_first_on_path_when_the_profile_resets_it, ::the_users_startup_files_still_run`
- UC-07C. Deploy over real SSH installs both programs on a fresh host, prints connection commands and its token, and upgrades a running server without losing its pane; a damaged archive leaves the host unchanged. → `uc_07c.py::deploy_installs_and_starts_tilrd_on_a_host_reached_over_ssh, ::a_failed_deploy_leaves_the_remote_as_it_was`
- UC-077. Exec по протоколу (без шелла): stdout и stderr раздельно, код выхода, сигнал, таймаут, отмена, большой вывод, stdin; параллельные exec; перезагрузка и ушедший клиент. → `uc_077.py::exec_returns_stdout_and_stderr_apart_and_the_exit_code, ::exec_runs_no_shell_so_nothing_is_expanded, ::exec_reports_the_signal_that_killed_the_program, ::exec_with_a_time_limit_is_killed_and_leaves_nothing_behind, ::a_cancelled_exec_ends_at_once_with_its_children, ::a_large_output_arrives_whole_and_in_order, ::stdin_given_to_exec_reaches_the_program, ::eight_execs_run_at_once, ::a_reload_ends_the_running_execs_with_their_children_and_says_why, ::a_client_that_goes_takes_its_execs_and_their_children_with_it, ::exec_start_to_a_server_with_exec_off_is_refused_with_the_reason`. Был красным до 2026-10-17 (Exec не было): теперь `ExecStart`/`ExecInput`/`ExecSignal`/`ExecCancel` → `ExecStarted`/`ExecOutput`/`ExecInputAck`/`ExecExit` в tilr.proto (`crates/tilrd/src/exec.rs`); программа — лидер своей группы, группа убивается по лимиту, отмене, выходу программы, уходу клиента и перезагрузке (клиент слышит `reason` `reload`); вывод — полосой массовых данных с обратным давлением; сервер с `--no-exec` отвечает на `ExecStart` ошибкой `exec is turned off` (с 2026-10-18)
- UC-079. Ввод: Ctrl-коды, Alt, функциональные клавиши, вставка (bracketed paste), мышь в приложении; Ctrl+C на русской раскладке даёт `^C`. → `uc_079.py::ctrl_alt_function_and_cursor_keys_reach_the_program_as_xterm_sends_them, ::bracketed_paste_wraps_the_text_and_cannot_be_ended_by_the_text, ::a_paste_without_bracketed_mode_is_plain_text, ::the_mouse_reaches_an_application_that_asked_for_it_and_no_other, ::ctrl_c_on_a_russian_layout_is_ctrl_c`. Был красным до 2026-10-17 (Ctrl+С на русской раскладке не давал ничего): теперь в `KeyInput.code` физическая клавиша (как W3C `KeyboardEvent.code`); с Ctrl и не-ASCII символом сервер кодирует US-символ клавиши, ASCII-символ (Dvorak, AZERTY) решает сам; Alt шлёт ESC и сам символ (Alt+ф — `ESC ф`, Alt+Shift+1 — `ESC !`, до 2026-10-18 было `ESC a`), составной символ (AltGr, Option+c → `ç`) без Alt идёт как есть; драйвер шлёт `layout`
- UC-07A. Cloud: `tunnel_open` / `tunnel_close` по управляющему каналу (Cloud играет тест) — `cloudflared` запущен и остановлен; команды, кроме туннельных, от Cloud не принимаются (модель доверия: Cloud не трогает панели). → `uc_07a.py::tunnel_open_starts_cloudflared_at_this_server_and_tunnel_close_stops_it, ::nothing_but_tunnel_commands_from_cloud_is_acted_on, ::a_server_without_authentication_gets_no_tunnel`
- UC-078. Настоящий `ssh -A` через клиент: `ssh-add -l` в панели видит ключи клиента; `git clone` по ssh через проброшенный агент. → `uc_078.py::ssh_add_in_a_pane_lists_the_keys_of_the_clients_agent, ::git_clone_over_ssh_uses_the_forwarded_agent_and_ssh_a_forwards_it_further, ::without_the_clients_agent_the_clone_is_refused` (хосты `laptop`, `git`)

## H. Нагрузка и безопасность

- UC-080. 200 панелей, 50 клиентов — задержка ввода p99 в пределах X.
- UC-081. Шторм вывода (`yes`, `cat /dev/urandom | base64`) в 10 панелях — сервер не падает, кадры сливаются, Ctrl+C работает.
- UC-082. Огромный `Resize` (65535×65535, и больше) — не отказ: размер приводится к 1..65535 (`crates/tilrd/src/pane.rs`), панель жива.
- UC-083. Сообщение 64 МБ / бесконечные очереди — лимит, соединение закрыто, остальные живы.
- UC-084. Перебор токена — попытки видны в журнале; без токена ничего, кроме отказа.
- UC-085. Чужой Origin, DNS rebinding на `/metrics` и `/socks`.
- UC-086. Шелл, игнорирующий SIGHUP, при остановке сервера — не остаётся сиротой. → `uc_086.py::a_shell_ignoring_sighup_does_not_outlive_the_server` (программа панели и foreground-задача с `trap '' HUP` убиты к возврату `tilr kill-server`; `nohup`-задача в фоне жива, больше ничего от сервера не осталось)
- UC-087. Soak: сутки работы с переподключениями и обновлениями — память и fd ровные.
- UC-088. `cloudflared` после SIGKILL сервера не остаётся. → `uc_088.py::cloudflared_is_gone_after_the_server_is_sigkilled` (`cloudflared` в своей process group; после SIGKILL сервера получает SIGTERM (parent-death signal, Linux) и уходит за 5 с)

## Отложено

### S. Синхронизация каталогов (rsync-like) — не делаем пока

Фичи в tilrd нет (только `Fs*`). Решить сначала: часть протокола tilrd или
отдельное ядро на QUIC (заметка «tilr — живая синхронизация Mac↔iPad»).

- Первая синхронизация пустого/непустого, слияние двух каталогов.
- Правка с одной стороны → на другой, задержка (цель ≤10 мс в LAN).
- Обрыв посреди синка → догон по индексу без повторной передачи всего.
- Конфликт (обе стороны правили) → копия-конфликт, ничего не потеряно.
- Удаление и переименование распространяются, не воскресают после reconnect.
- Большой файл, маленькая правка → передан только изменённый блок.
- `A.txt`/`a.txt` Linux↔macOS, кириллица NFD/NFC, симлинки, права.
- Лимит inotify `max_user_watches` на большом дереве.
- На приёмнике не бывает недописанного файла.
