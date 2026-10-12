# Сценарии использования и тесты

Каждый сценарий — тест, который поднимает настоящий сервер, подключается к нему
по WebSocket как клиент и смотрит, что видит шелл в панели. Тесты лежат в
`crates/tilr-server/tests/`, общая обвязка — в `tests/common/mod.rs`.

Запуск: `cargo test --workspace --features tilr-server/failpoints`, на Linux —
`scripts/linux.sh`. Сценарии со сбоем посередине (помечены †) устраивают сбой
в заданном месте кода через точки отказа (`src/fail.rs`) и собираются только
с этой фичей.

Имя теста и есть формулировка сценария.

## Соединение и панели — `tests/panes.rs`

| Сценарий | Тест |
|---|---|
| Клиент здоровается, сервер сообщает версию, возможности и лимит буфера | `hello_announces_the_clipboard_and_its_limit` |
| Клиент с другой версией протокола (новее, старее, без версии) получает ошибку с обеими версиями, соединение закрывается | `a_client_built_for_another_protocol_version_is_turned_away` |
| Первое, что получает клиент, — ответ на его `Hello`: списки панелей до этого ему не шлются, после — как всем | `the_first_thing_a_client_hears_is_the_answer_to_its_hello` |
| Клиент, не начавший с `Hello`, получает ошибку и отключается; его ввод до шелла не доходит | `a_client_that_skips_hello_is_turned_away` |
| Страница в браузере подключается только с разрешённого адреса; программы без `Origin` проходят | `a_browser_page_connects_only_from_an_allowed_origin` |
| Команда в панели выполняется, виден её вывод; код выхода шелла доходит до клиента | `a_pane_runs_commands_and_reports_how_its_shell_exited` |
| Панель открывается в домашнем каталоге, если клиент не назвал каталог или назвал несуществующий на сервере; в названном — если он есть | `a_pane_starts_in_the_home_directory_unless_given_one_that_exists` |
| Закрытие панели завершает шелл | `closing_a_pane_ends_its_shell` |
| Два клиента смотрят одну панель и оба печатают в неё | `two_clients_watch_and_type_into_the_same_pane` |
| Клиент ушёл — панель живёт, остальным приходит обновлённый список | `a_client_that_leaves_does_not_take_the_pane_with_it` |
| Клиент отписался — кадры ему больше не идут | `a_client_that_stops_watching_gets_no_more_frames` |
| Изменение размера доходит до терминала, который видит шелл | `resizing_a_pane_resizes_the_terminal_the_shell_sees` |
| Незнакомый тип сообщения игнорируется | `a_message_type_the_server_does_not_know_is_ignored` |
| Битый пакет — ошибка, соединение остаётся | `a_malformed_packet_gets_an_error_and_the_connection_stays` |
| Обращение к несуществующей панели — ошибка; имя, которое не UUID, — тоже | `asking_for_a_pane_that_does_not_exist_gets_an_error` |
| У панели свой UUID: шелл видит его в `$TILR_PANE`, процесс панели — в командной строке (`pane --id <uuid>`), размер там указан, только если он не 80×24 | `a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` |
| `tilr-server ps` показывает панели работающего сервера: id, pid, размер, команду | `ps_lists_the_panes_of_a_running_server` |
| У сервера свой id, после перезапуска тот же; у другого сервера — другой | `a_server_keeps_its_id_when_it_restarts` |

## Доступ — `tests/auth.rs`

| Сценарий | Тест |
|---|---|
| Клиент без токена доступа или с чужим получает ошибку и отключается; с токеном — проходит | `a_client_needs_the_access_token` |
| Токен лежит в файле, доступном только пользователю, переживает перезапуск; `tilr-server token` печатает его же | `the_token_is_kept_for_the_user_only_and_across_restarts` |
| Сервер с `--no-auth` токен не спрашивает и предупреждает об этом в журнале | `a_server_started_without_authentication_asks_for_no_token` |

## tilr Cloud — `tests/cloud.rs`

Cloud в этих тестах играет сам тест, `cloudflared` — скрипт, который
записывает, как его запустили. Сам Cloud проверяется своими тестами
(`../cloud/test`).

| Сценарий | Тест |
|---|---|
| Привязанный сервер подключается к Cloud со своим токеном; получив туннель, запускает `cloudflared`, направленный только на свой порт; туннель отозван — `cloudflared` остановлен, секрет удалён | `a_linked_server_gets_its_tunnel_from_cloud` |
| Сервер с `--no-auth` от туннеля отказывается | `a_server_without_authentication_takes_no_tunnel` |
| Описание туннеля, которое дописало бы конфиг `cloudflared` (другой сервис), отклоняется | `a_tunnel_that_would_lead_elsewhere_is_refused` |
| При перезагрузке сервера старый `cloudflared` останавливается, преемник запускает свой | `a_reload_replaces_cloudflared_with_one_of_the_new_server` |
| Сервер, привязанный на ходу, подключается сам; отвязанный — отключается | `a_server_linked_while_it_runs_connects_on_its_own` |

## Переподключение — `tests/reconnect.rs`

| Сценарий | Тест |
|---|---|
| Клиент вернулся: панель на месте, первый кадр — весь экран, шелл тот же | `a_client_that_reconnects_finds_its_pane_as_it_left_it` |
| То, что напечатано, пока никто не смотрел, видно после возврата | `output_printed_while_nobody_watched_is_there_on_reconnect` |
| Повторная подписка присылает весь экран заново | `subscribing_again_sends_the_whole_screen_again` |
| Много переподключений не оставляют «призрачных» зрителей | `many_reconnects_leave_no_viewers_behind` |
| Клиент пропал молча (соединение открыто, на пинги не отвечает): сервер сам его отключает, остальным приходит обновлённый список; вернувшись, клиент переподключается к своей панели | `a_client_that_vanishes_without_closing_is_dropped` |
| Клиент молчит, но на месте (отвечает на пинги): соединение остаётся | `an_idle_client_that_is_still_there_stays_connected` |
| С `--client-timeout 0` молчащих клиентов не отключают | `a_silent_client_is_kept_when_the_timeout_is_off` |
| Сервер пропал молча: клиент шлёт `Ping`, ответа `Pong` нет — клиент это видит; сервер ожил — ответ приходит | `a_client_can_tell_that_the_server_vanished_without_closing` |

## Прокрутка — `tests/scroll.rs`

| Сценарий | Тест |
|---|---|
| Прокрутка назад показывает историю, вперёд — возвращает | `scrolling_back_shows_history_and_scrolling_forward_returns` |
| «В самый низ» возвращает живой экран | `to_bottom_returns_to_the_live_screen` |
| Прокрутка упирается в начало и в конец истории | `scrolling_stops_at_both_ends_of_history` |
| Нажатие клавиши возвращает вид вниз | `typing_returns_the_view_to_the_live_screen` |
| Позиция прокрутки общая для всех, кто смотрит панель | `the_scroll_position_is_shared_by_everyone_watching` |
| Подключившийся к прокрученной панели видит прокрученный вид | `a_client_joining_a_scrolled_pane_sees_the_scrolled_view` |
| Новый вывод не сдвигает вид, пока читают историю | `output_arriving_while_scrolled_back_does_not_move_the_view` |
| История на месте после переподключения | `history_is_still_there_after_reconnecting` |
| История переживает перезагрузку сервера | `history_survives_a_reload` |
| Прокрученный вид остаётся прокрученным после перезагрузки | `a_scrolled_view_stays_scrolled_through_a_reload` |

## Обновление с CDN — `tests/update.rs`

| Сценарий | Тест |
|---|---|
| `tilr-server update` ставит последний релиз для своей платформы; работающий сервер сам перезагружается, шелл цел | `update_installs_the_latest_release_and_the_running_server_keeps_its_shells` |
| Последний релиз уже стоит — ничего не скачивается, файл не тронут | `update_does_nothing_when_the_latest_release_is_the_one_installed` |
| Скачанное не сходится с контрольной суммой, не запускается или релиза для платформы нет — ничего не установлено, мусора рядом нет | `update_installs_nothing_that_fails_its_checksum_or_does_not_run` |
| CDN подводит: скачивание оборвалось посередине, релиз назван, а файлов ещё нет, вместо CDN отвечает чужая страница, никто не отвечает — ничего не установлено | `update_installs_nothing_when_the_cdn_misbehaves` |
| В каталог с бинарём нельзя писать — ничего не установлено | `update_installs_nothing_when_it_cannot_write_next_to_the_executable` |
| † Апдейтер убит за миг до подмены: бинарь прежний; следующее обновление проходит и убирает недокачанное | `an_update_killed_before_the_swap_changes_nothing_and_the_next_one_tidies_up` |
| † Два обновления разом: релиз ставится один раз, второе ждёт и видит, что всё уже стоит | `two_updates_at_once_install_the_release_once` |

### Живая проверка на машине — `scripts/update-test.sh`

Автотесты выше ходят на подставной CDN. Этот протокол проверяет настоящий:
ставит на машину старый релиз, открывает панели и обновляет до последнего.

```sh
scripts/update-test.sh <ssh-хост> <старая-версия>
scripts/update-test.sh ubu-test 0.1.0-7b5740b
```

Нужны: доступ по ssh, на машине — `curl` и `sha256sum`; локально — собранный
`test-client`. Тестовый сервер живёт в `~/tilr-update-test` на своём порту,
установленный tilr не трогает и за собой убирает. На каждую проверку — строка
`ok` или `not ok`, код выхода 1, если что-то не сошлось.

| № | Сценарий | Что проверяется |
|---|---|---|
| 1 | Установка вручную | Старый релиз скачивается, сходится с контрольной суммой, запускается и называет свою версию |
| 2 | Обновление из панели, открыты четыре панели | Версия стала последней; pid сервера тот же; перезагрузка одна, со всеми панелями; шелл тот же, переменная и фоновая задача на месте; счётчик печатал без пропусков и пауз; полноэкранная программа осталась на экране и закрывается; панель с командой вместо шелла жива |
| 3 | Обновление ещё раз | «Уже последняя», перезагрузки нет |
| 4 | CDN недоступен | Ошибка; бинарь не тронут, рядом ничего не осталось, сервер работает |
| 5 | Откат вручную и обновление снова, панели открыты | Панели переживают путь назад и вперёд; pid сервера тот же; счётчик без пропусков за все три перезагрузки; в логе нет отказов и ошибок |
| 6 | Обновление при остановленном сервере | Обновление проходит; сервер стартует уже новой версией и открывает панель |

Не покрыто: клиент, подключённый в момент обновления (переподключение
проверяют автотесты); история прокрутки после обновления; релиз с другим
форматом состояния; платформы кроме `linux-aarch64`.

## Обновление без потери шеллов — `tests/reload.rs`

| Сценарий | Тест |
|---|---|
| Перезагрузка по сигналу: шелл тот же, экран цел, сервер работает дальше | `panes_survive_a_reload_asked_for_with_a_signal` |
| Замена бинаря на диске: сервер замечает сам и перезагружается | `panes_survive_the_executable_being_replaced_on_disk` |
| С `--no-auto-reload` замена бинаря игнорируется | `a_replaced_executable_is_ignored_when_auto_reload_is_off` |
| Нерабочий новый бинарь: перезагрузки нет, сервер продолжает на старом коде | `a_broken_executable_is_not_reloaded_into` |
| Новый бинарь читает другой формат состояния (супервизора или панели): перезагрузки нет, шеллы целы; подходящий бинарь после этого подхватывается | `an_executable_that_reads_another_state_format_is_not_reloaded_into` |
| После перезагрузки процессы сервера и панелей выглядят в `ps` так же, как до неё; файлы с переданным состоянием прочитаны и удалены | `a_reload_shows_in_no_command_line_and_leaves_no_file_behind` |
| Перезагрузка без панелей | `a_reload_with_no_panes_open_works` |
| Несколько панелей: у каждой свой шелл и своё состояние | `every_pane_keeps_its_own_shell_across_a_reload` |
| Вывод, идущий во время перезагрузки, не теряется | `output_printed_during_a_reload_is_not_lost` |
| Ввод прямо перед перезагрузкой доходит до шелла, один раз | `input_typed_just_before_a_reload_reaches_the_shell` |
| Несколько перезагрузок подряд | `a_server_can_reload_again_and_again` |
| Шелл, вышедший после перезагрузки, сообщает код выхода | `a_shell_that_exits_after_a_reload_still_reports_its_exit_code` |
| Заголовок, папка и время создания панели сохраняются | `the_title_and_directory_of_a_pane_are_kept_across_a_reload` |
| Остановка сервера завершает шеллы | `stopping_the_server_ends_the_shells` |
| То же после перезагрузки | `stopping_the_server_after_a_reload_ends_the_shells` |
| † Перезагрузка не удалась до передачи управления: всё как было, даже соединения клиентов целы | `a_reload_that_fails_before_the_handover_leaves_everything_as_it_was` |
| † Процесс панели не смог перезагрузиться: панель работает дальше на старом коде | `a_pane_that_cannot_hand_over_carries_on_in_the_code_it_has` |
| † Процесс панели упал, принимая управление: эта панель потеряна (шелл завершён), остальные целы | `a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on` |
| † Новая версия упала, приняв управление: сервер и все шеллы потеряны — предел перезагрузки на месте, назад пути нет; осиротевших шеллов не остаётся | `a_version_that_crashes_taking_over_ends_the_server_and_its_shells` |
| † Сервер убит посреди перезагрузки: шеллы завершаются, осиротевших не остаётся | `a_server_killed_in_the_middle_of_a_reload_leaves_no_shell_behind` |
| † Бинарь заменили ещё раз, пока шла перезагрузка: ни один шелл не потерян | `an_executable_replaced_again_during_a_reload_costs_no_shell` |

## Буфер обмена — `tests/clipboard.rs`

| Сценарий | Тест |
|---|---|
| Вставленная картинка доходит до программы командами Claude Code, байт в байт | `a_pasted_image_reaches_a_program_that_reads_the_clipboard` |
| Буфер общий для всех панелей, следующая вставка его заменяет | `the_clipboard_is_shared_by_all_panes_and_replaced_by_the_next_paste` |
| Слишком большая загрузка отклоняется, старая картинка не всплывает | `an_upload_over_the_limit_is_refused_and_leaves_no_stale_content` |
| Буфер выключается флагом | `the_clipboard_can_be_turned_off` |
| Содержимое буфера истекает | `pasted_content_expires` |

## Файлы и `tilr edit` — `tests/files.rs`

Файловый протокол (`Fs*`, возможность `"files"`) и события с хоста
(`HostEvent`, возможность `"host-events"`): программа в панели просит клиента
что-то открыть через сокет сервера.

| Сценарий | Тест |
|---|---|
| Клиент видит возможность `files` и лимит; листинг каталога: путь без ссылок, записи по имени, тип, размер, права, время; ссылка показана ссылкой с типом цели, битая — `other`; `stat` ссылку не разыменовывает, корень смотреть можно; листинг через ссылку и через `..`; ошибки — в ответе на запрос | `a_client_lists_a_directory_and_looks_at_its_entries` |
| Файл читается кусками целиком и частями (offset/length); за концом — пусто; больше лимита за раз — отказ, частями — можно | `a_file_is_read_in_chunks_whole_or_in_part` |
| Запись кусками: новый файл (`create`), замена (`truncate`, права сохраняются), запись сквозь ссылку; уже есть / нет файла / каталог / больше лимита / данных меньше или больше заявленного — отказ, файл не тронут; клиент ушёл посреди записи — файл прежний, временного файла нет | `a_file_is_written_whole_or_not_at_all` |
| Каталог создаётся; переименование файла и каталога, поверх существующего — отказ; удаление ссылки не трогает цель; непустой каталог удаляется только с `recursive` | `directories_are_made_entries_renamed_and_deleted` |
| Наблюдение за каталогом: создание, изменение, удаление приходят `FsChanged`; сохранение через протокол — одно изменение файла, временный файл не виден; подкаталоги не наблюдаются; после `FsUnwatch` изменений нет | `a_watched_directory_reports_what_changes_in_it` |
| Вне `roots` ничего: `..`, ссылка наружу (листинг, чтение, запись, наблюдение), чужой путь — «outside», даже существование снаружи не выдаётся; корень нельзя удалить или переместить; саму ссылку внутри — можно | `nothing_outside_the_shared_directories_is_served` |
| `readonly = true`: смотреть и читать можно, запись, mkdir, rename, delete — отказ, на диске ничего не меняется | `a_read_only_server_shows_files_but_changes_nothing` |
| `--no-files` и `[files] enabled = false`: возможности нет, `fs_max_bytes` 0, запросы без ответа и без последствий, соединение живо | `files_can_be_turned_off` |
| В панели `$TILR_PANE`, `$TILR_SERVER` (id сервера) и `$TILR_SOCK` (сокет); `tilr edit notes.md` доходит до всех клиентов, смотрящих панель, как `HostEvent` с абсолютным путём; `tilr open URL -- args`, `--kind`; прочие команды `tilr` уходят следующему `tilr` на PATH | `tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file` |
| Никто не смотрит панель — «no tilr client attached», код 1; вне панели — «not inside a tilr pane», код 1; чужая панель — «no such pane»; не-JSON — `bad request`; остановленный сервер убирает сокет | `tilr_edit_fails_with_no_client_watching_or_outside_a_pane` |

## SOCKS-прокси — `tests/socks.rs`

| Сценарий | Тест |
|---|---|
| Страница по WebSocket `/socks` с токеном в `Authorization: Bearer` доходит до сервиса на loopback сервера (IPv4), туда и обратно, в том числе больше одного сообщения | `a_page_reaches_a_loopback_service_through_the_socks_websocket` |
| Токен в `?token=`; имя `localhost` резолвится на сервере | `a_page_that_cannot_set_headers_gives_the_token_in_the_query` |
| `pb.localhost`, `nonexistent.LocalHost` — loopback сервера, без DNS | `any_name_under_localhost_is_the_servers_loopback` |
| Без токена или с чужим — 401; сервер с `--no-auth` токен не спрашивает | `the_socks_websocket_needs_the_access_token` |
| Страница с чужого origin — 403, как у основного протокола | `the_socks_websocket_comes_only_from_an_allowed_origin` |
| Без `--socks` прокси нет: `/socks` — 404, сырой SOCKS отключается, основной протокол как был | `the_proxy_is_off_unless_asked_for` |
| `socks = true` в файле конфигурации включает прокси | `the_configuration_file_turns_the_proxy_on` |
| Сырой SOCKS5 на том же порту: пароль — токен; чужой пароль — 0x01; без пароля — 0xFF | `a_raw_socks_client_gives_the_token_as_its_password` |
| Сырой SOCKS5 сервера с `--no-auth` пароля не спрашивает | `a_raw_socks_client_of_a_server_without_authentication_needs_no_password` |
| BIND — 0x07; закрытый порт — 0x05, затем WebSocket закрывается с 1000 | `only_connect_is_served_and_a_refusal_is_reported` |
| Текстовое сообщение закрывает WebSocket с 1003 | `a_text_message_closes_the_socks_websocket` |
| Несколько сессий идут параллельно, недоговорившая никого не держит, терминал работает | `socks_sessions_run_side_by_side_with_the_terminal` |

## SSH-агент — `tests/agent.rs`

Проброс агента как у `ssh -A` (возможность `"agent"`): у каждой панели свой
сокет в `SSH_AUTH_SOCK`, программа говорит через него с агентом на стороне
клиента потоком `StreamOpen` / `StreamData` / `StreamClose`.

| Сценарий | Тест |
|---|---|
| Сервер объявляет возможность `agent`; с `--no-agent` и `agent = false` её нет, сокета нет, `SSH_AUTH_SOCK` в панели пуст | `the_agent_is_forwarded_unless_turned_off` |
| В панели `SSH_AUTH_SOCK` указывает на её собственный сокет (`agent/<pane>.sock`, 0600, каталог 0700); у каждой панели свой; панель закрылась — сокета нет | `a_pane_has_ssh_auth_sock_pointing_at_its_own_socket` |
| Программа подключилась — клиент получает `StreamOpen` (`ssh-agent`, панель); байты идут в обе стороны, больше одного сообщения тоже; программа закрыла — `StreamClose`; клиент закрыл — программа дочитывает посланное и получает конец; данные закрытому или чужому потоку игнорируются | `a_program_talks_to_the_agent_of_the_client_through_the_panes_socket` |
| Панель смотрит клиент без `agent`, клиент с `agent` её не смотрит или отписался — соединение сразу закрывается, клиентам ничего не приходит | `with_no_agent_client_attached_the_program_finds_no_agent` |
| Клиент агента отключился посреди потока — программа читает конец; следующий клиент обслуживает тот же сокет | `the_program_reads_the_end_when_the_agent_client_disconnects` |
| Два клиента с `agent`: поток получает подписавшийся последним (клиент без `agent` не в счёт); он отписался — получает первый | `the_client_that_subscribed_last_serves_the_agent` |
| Перезагрузка: открытый поток кончается, сокет на том же пути снова принимает и ведёт к переподключившемуся клиенту | `the_panes_socket_works_after_a_reload` |

## Метрики — `tests/metrics.rs`

Задержку ввода клиент меряет сам, без синхронизации часов: помечает ввод
`ClientMessage.seq` = `HelloAck.seq_base` + n и ждёт кадр, где
`Frame.last_input_seq` — этот номер (последний зачтённый ввод, от любого
клиента); в кадре `Frame.timing` — время внутри процесса панели. Счётчики
сервера — `GET /metrics`.

| Сценарий | Тест |
|---|---|
| До ввода `last_input_seq` = 0; кадры после ввода с `seq` несут его, `timing` есть всегда, `build_us` > 0, кадр с выводом — `output_bytes` и `reads`; ввод без `seq` номер не сбивает; `Scroll` с `seq`, ничего не сдвинувший, получает свой кадр; клиент, нумерующий с 1 без `seq_base`, работает | `a_frame_names_the_input_it_shows_and_how_it_was_made` |
| Ввод, ничего не изменивший на экране (клик без отслеживания мыши, пустой `RawInput`), — кадр без строк с его `seq`, тем же курсором, размером и режимами; ввод без `seq` кадра не даёт | `input_that_changes_nothing_is_answered_with_an_empty_frame` |
| Переподключение (перезагрузка страницы): новый `seq_base` больше старого; полный кадр несёт номер старого соединения, не свой; ввод нового — его номер | `a_client_that_reconnects_tells_its_input_from_before` |
| Два клиента на одной панели: у каждого свой `seq_base`, оба видят номера обоих по порядку | `two_clients_on_one_pane_each_see_their_own_input` |
| `Scroll` зачитывается сразу, хотя ввод с клавиатуры ещё ждёт pty (1 МиБ в pty без читателя) | `a_scroll_is_credited_while_key_input_waits_for_the_pty` |
| Перезагрузка сервера: `last_input_seq` и ввод, ждущий pty, переживают её, ввод зачитывается после, `input_us` — с приёма до перезагрузки; номера соединений продолжаются | `the_last_input_and_input_still_waiting_survive_a_reload` |
| `/metrics` без токена, с чужим (в `?token=` и в `Authorization`) — 401; чужой origin — 403; с токеном любым из двух способов — 200, `text/plain; version=0.0.4`; основной протокол на том же порту как был | `metrics_ask_for_the_access_token` |
| Сервер с `--no-auth` токена для `/metrics` не спрашивает у `Host` localhost, `127.0.0.1`, `[::1]` (с портом и без) и `Sec-Fetch-Site` не cross-site | `metrics_need_no_token_on_a_server_without_one` |
| Без токена: `Sec-Fetch-Site: cross-site`, чужой `Host` (DNS rebinding) или без `Host` — 403; с токеном решает токен | `without_a_token_metrics_are_for_this_machine_only` |
| Запрос, пришедший кусками (строка запроса и пустая строка разорваны) | `a_metrics_request_may_come_in_pieces` |
| После ввода и кадров: счётчики кадров, байтов и ввода панели, гистограммы маршрута и лага цикла, CPU и память супервизора и процесса панели | `metrics_count_frames_input_and_routing` |

## Не покрыто

- Клиент: поведение веб-клиента при обрыве (автопереподключение, возврат в ту же
  панель) проверяется только руками. `Ping` веб-клиент пока не шлёт.
- Медленный клиент, которого сервер отключает за отставание.
- Перезагрузка, когда в панели открыто полноэкранное приложение (vim, htop):
  есть юнит-тесты снимка экрана, сквозного сценария нет.
- Перезагрузка в момент создания панели.
- Обновление, когда на диске кончилось место: недокачанный файл, запись
  состояния при перезагрузке. Проверен только запрет записи в каталог, и тот
  тест не выполняется от root (в контейнере).
- Новая версия падает уже после передачи управления: сервер и шеллы теряются
  (тест это фиксирует). Защиты, кроме проверки до перезагрузки, нет.
- Обновление между версиями с разным форматом состояния без перезапуска:
  сейчас такая перезагрузка отклоняется (шеллы целы), переноса состояния из
  старого формата в новый нет.
- Клавиши и мышь: кодирование проверено юнит-тестами экрана, сквозных сценариев
  (Ctrl+C прерывает команду, мышь в приложении) нет.
- Прокрутка при изменении размера панели.
- tilr Cloud: `tilr-server cloud login` / `logout` проверены только руками
  против локального Cloud (`wrangler dev`). Настоящий `cloudflared` и настоящий
  туннель Cloudflare не проверялись ни разу.
- `cloudflared`, оставшийся после аварийной смерти сервера (SIGKILL), никто не
  убирает.
- Файлы: перезагрузка сервера посреди записи оставляет временный файл
  `.<имя>.tilr-write-…` рядом (без перезагрузки он убирается); наблюдения
  после перезагрузки клиент ставит заново, как и подписки.
- `tilr edit` доставляется и старому клиенту, который `HostEvent` не знает и
  молча игнорирует: сервер считает его получателем.
- SSH-агент: сокеты панелей, оставшиеся после аварийной смерти сервера
  (SIGKILL), никто не убирает. Настоящий `ssh` с настоящим агентом сквозь
  клиент не проверялся: тесты играют роль и программы, и клиента.
