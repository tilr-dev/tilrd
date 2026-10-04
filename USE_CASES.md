# Сценарии использования и тесты

Каждый сценарий — тест, который поднимает настоящий сервер, подключается к нему
по WebSocket как клиент и смотрит, что видит шелл в панели. Тесты лежат в
`crates/tilr-server/tests/`, общая обвязка — в `tests/common/mod.rs`.

Запуск: `cargo test --workspace`, на Linux — `scripts/linux.sh`.

Имя теста и есть формулировка сценария.

## Соединение и панели — `tests/panes.rs`

| Сценарий | Тест |
|---|---|
| Клиент здоровается, сервер сообщает версию, возможности и лимит буфера | `hello_announces_the_clipboard_and_its_limit` |
| Клиент с другой версией протокола (новее, старее, без версии) получает ошибку с обеими версиями, соединение закрывается | `a_client_built_for_another_protocol_version_is_turned_away` |
| Клиент, не начавший с `Hello`, получает ошибку и отключается; его ввод до шелла не доходит | `a_client_that_skips_hello_is_turned_away` |
| Страница в браузере подключается только с разрешённого адреса; программы без `Origin` проходят | `a_browser_page_connects_only_from_an_allowed_origin` |
| Команда в панели выполняется, виден её вывод; код выхода шелла доходит до клиента | `a_pane_runs_commands_and_reports_how_its_shell_exited` |
| Закрытие панели завершает шелл | `closing_a_pane_ends_its_shell` |
| Два клиента смотрят одну панель и оба печатают в неё | `two_clients_watch_and_type_into_the_same_pane` |
| Клиент ушёл — панель живёт, остальным приходит обновлённый список | `a_client_that_leaves_does_not_take_the_pane_with_it` |
| Клиент отписался — кадры ему больше не идут | `a_client_that_stops_watching_gets_no_more_frames` |
| Изменение размера доходит до терминала, который видит шелл | `resizing_a_pane_resizes_the_terminal_the_shell_sees` |
| Незнакомый тип сообщения игнорируется | `a_message_type_the_server_does_not_know_is_ignored` |
| Битый пакет — ошибка, соединение остаётся | `a_malformed_packet_gets_an_error_and_the_connection_stays` |
| Обращение к несуществующей панели — ошибка | `asking_for_a_pane_that_does_not_exist_gets_an_error` |

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

## Обновление без потери шеллов — `tests/reload.rs`

| Сценарий | Тест |
|---|---|
| Перезагрузка по сигналу: шелл тот же, экран цел, сервер работает дальше | `panes_survive_a_reload_asked_for_with_a_signal` |
| Замена бинаря на диске: сервер замечает сам и перезагружается | `panes_survive_the_executable_being_replaced_on_disk` |
| С `--no-auto-reload` замена бинаря игнорируется | `a_replaced_executable_is_ignored_when_auto_reload_is_off` |
| Нерабочий новый бинарь: перезагрузки нет, сервер продолжает на старом коде | `a_broken_executable_is_not_reloaded_into` |
| Новый бинарь читает другой формат состояния (супервизора или панели): перезагрузки нет, шеллы целы; подходящий бинарь после этого подхватывается | `an_executable_that_reads_another_state_format_is_not_reloaded_into` |
| Перезагрузка без панелей | `a_reload_with_no_panes_open_works` |
| Несколько панелей: у каждой свой шелл и своё состояние | `every_pane_keeps_its_own_shell_across_a_reload` |
| Вывод, идущий во время перезагрузки, не теряется | `output_printed_during_a_reload_is_not_lost` |
| Ввод прямо перед перезагрузкой доходит до шелла, один раз | `input_typed_just_before_a_reload_reaches_the_shell` |
| Несколько перезагрузок подряд | `a_server_can_reload_again_and_again` |
| Шелл, вышедший после перезагрузки, сообщает код выхода | `a_shell_that_exits_after_a_reload_still_reports_its_exit_code` |
| Заголовок, папка и время создания панели сохраняются | `the_title_and_directory_of_a_pane_are_kept_across_a_reload` |
| Остановка сервера завершает шеллы | `stopping_the_server_ends_the_shells` |
| То же после перезагрузки | `stopping_the_server_after_a_reload_ends_the_shells` |

## Буфер обмена — `tests/clipboard.rs`

| Сценарий | Тест |
|---|---|
| Вставленная картинка доходит до программы командами Claude Code, байт в байт | `a_pasted_image_reaches_a_program_that_reads_the_clipboard` |
| Буфер общий для всех панелей, следующая вставка его заменяет | `the_clipboard_is_shared_by_all_panes_and_replaced_by_the_next_paste` |
| Слишком большая загрузка отклоняется, старая картинка не всплывает | `an_upload_over_the_limit_is_refused_and_leaves_no_stale_content` |
| Буфер выключается флагом | `the_clipboard_can_be_turned_off` |
| Содержимое буфера истекает | `pasted_content_expires` |

## Не покрыто

- Клиент: поведение веб-клиента при обрыве (автопереподключение, возврат в ту же
  панель) проверяется только руками. `Ping` веб-клиент пока не шлёт.
- Медленный клиент, которого сервер отключает за отставание.
- Перезагрузка, когда в панели открыто полноэкранное приложение (vim, htop):
  есть юнит-тесты снимка экрана, сквозного сценария нет.
- Перезагрузка в момент создания панели.
- Обновление между версиями с разным форматом состояния без перезапуска:
  сейчас такая перезагрузка отклоняется (шеллы целы), переноса состояния из
  старого формата в новый нет.
- Клавиши и мышь: кодирование проверено юнит-тестами экрана, сквозных сценариев
  (Ctrl+C прерывает команду, мышь в приложении) нет.
- Прокрутка при изменении размера панели.
- Аутентификация: её пока нет.
