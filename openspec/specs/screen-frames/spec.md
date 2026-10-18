# screen-frames Specification

## Purpose

Covers what a pane sends of its screen and how it takes input: `Frame` (full frames and diffs, rows, runs and styles, cursor, title, modes, `viewport_top`, kitty graphics), scrollback and `Scroll`, and the encoding of `KeyInput` (including the physical key in `KeyInput.code`), `MouseInput`, `Paste` and `RawInput` into the bytes the program reads. Creating and watching panes is in the `panes` spec; `Frame.last_input_seq` and `Frame.timing` in `metrics`; skipping frames for a lagging client in `metrics` and `client-liveness`; carrying the screen across a reload in `reload`.

## Requirements

### Requirement: Full frames and diffs
A `Frame` with `full` true SHALL carry every row of the viewport and replace the client's copy of the screen; with `full` false it SHALL carry only the rows changed since the pane's previous frame. Each `Row{y, runs}` SHALL replace viewport row `y` entirely, cells past its last run being blank; trailing blank cells SHALL be left out.

**Verified by**: unit tests `ghostty::tests::full_frame_has_every_row_and_trims_blanks` and `ghostty::tests::diff_frame_has_only_changed_rows`.

#### Scenario: Full frame of a short screen
- **WHEN** `hello` is printed on a 4-row screen and a full frame is built
- **THEN** the frame has 4 rows, row 0 reads `hello`, row 1 has no runs, and the cursor is visible at (5, 0)

#### Scenario: Diff
- **WHEN** after a frame showing `one` / `two` the program prints `!`
- **THEN** the next frame is not full and carries row 1 alone, reading `two!`

### Requirement: Every frame says the screen's shape and modes
Every frame, full or not, SHALL carry `cols`, `rows`, `cursor`, `mouse_tracking`, `alt_screen`, `cell_width_px`, `cell_height_px` and `viewport_top` as they are when it is built.

**Verified by**: unit test `ghostty::tests::scrollback_viewport` and `metrics::input_that_changes_nothing_is_answered_with_an_empty_frame`.

#### Scenario: A diff after a scroll
- **WHEN** the view is scrolled back by 3 rows
- **THEN** the next frame carries the new `viewport_top` and the cursor with `visible` false along with the changed rows

### Requirement: Runs and styles
Cells of a row SHALL be sent as `Run`s of consecutive cells sharing one style; `Run.style` SHALL be an index into the frame's `styles` plus one, 0 meaning the default style. `Style` SHALL carry `fg`, `bg`, `underline_color` (absent = default, else `palette` or `rgb` 0xRRGGBB), `attrs` (1 bold, 2 italic, 4 faint, 8 blink, 16 inverse, 32 invisible, 64 strikethrough, 128 overline) and `underline`.

**Verified by**: unit test `ghostty::tests::styles_are_split_into_runs`.

#### Scenario: A bold red letter between plain ones
- **WHEN** the program prints `a ESC[1;31m b ESC[0m c`
- **THEN** row 0 has three runs with styles 0, n, 0, and style n has `attrs` bold and `fg` palette 1

### Requirement: Wide cells and grapheme clusters
A run of two-column cells SHALL have `wide` set; a grapheme of several codepoints SHALL be one cell, sent as a run with `cluster` set whose text is the whole grapheme. A cell erased with a background color SHALL carry that color as its style's `bg`.

**Verified by**: unit test `ghostty::tests::wide_and_cluster_cells`.

#### Scenario: Wide and combined characters
- **WHEN** the program prints `a世b` and then `e` followed by U+0301
- **THEN** `世` is a run with `wide` set, and `é` is a run with `cluster` set and text `é`

### Requirement: When a pane sends a frame
A pane SHALL send a frame when its screen, cursor, title, modes, image placements or viewport position changed, and none when nothing did (credited input aside, see `metrics`). Frames SHALL be at least 8 ms apart, faster output being coalesced. `Frame.seq` SHALL grow by one per frame of the pane, and a frame SHALL go to every viewer of the pane.

**Verified by**: `metrics::input_that_changes_nothing_is_answered_with_an_empty_frame` and unit test `ghostty::tests::cursor_move_alone_produces_frame`.

#### Scenario: A cursor move is a frame
- **WHEN** the program only moves the cursor (`ESC[D`)
- **THEN** a frame is sent whose cursor `x` moved left by one

#### Scenario: Nothing changed
- **WHEN** no output came and no input was credited since the last frame
- **THEN** the pane sends no frame

### Requirement: When a frame is full
A full frame SHALL be sent after a `Subscribe` (to every viewer of the pane), after a `Resize` that changed the size or the cell, after a reload (see `reload`) and for a lagging client catching up (see `metrics`).

**Verified by**: `reconnect::subscribing_again_sends_the_whole_screen_again`, `reconnect::a_client_that_reconnects_finds_its_pane_as_it_left_it` and `panes::resizing_a_pane_resizes_the_terminal_the_shell_sees`.

#### Scenario: Subscribing again
- **WHEN** a client that already watches a pane sends `Subscribe` for it again
- **THEN** its next frame of the pane is full

#### Scenario: Resize
- **WHEN** a client resizes a pane to 50 x 10
- **THEN** the next frame is full with `cols` 50 and `rows` 10

### Requirement: The cursor
`Frame.cursor` SHALL carry `visible`, `x`, `y` (viewport cell, zero-based), `shape` (block, bar, underline, hollow block) and `blinking`. The cursor SHALL be reported not visible when the program hid it or when its cell is not in the viewport (the view is scrolled back).

**Verified by**: unit tests `ghostty::tests::full_frame_has_every_row_and_trims_blanks` and `ghostty::tests::scrollback_viewport`.

#### Scenario: Scrolled away from the cursor
- **WHEN** the view is scrolled back 3 rows
- **THEN** the frame's cursor has `visible` false

### Requirement: The title in frames
`Frame.title` SHALL be present on every full frame and on a diff frame only when the program changed the title (OSC 0/2) since the previous frame; absent means the title is unchanged.

**Verified by**: unit test `ghostty::tests::title_reported_on_change`.

#### Scenario: Title change
- **WHEN** after a full frame the program prints `ESC ] 2 ; hi BEL`
- **THEN** the next frame has `title` `hi`

### Requirement: Terminal modes in frames
`Frame.mouse_tracking` SHALL be true while the program has asked for mouse events, and `Frame.alt_screen` true while the alternate screen is active; a change of either SHALL make a frame.

**Verified by**: unit test `ghostty::tests::mouse_encoded_only_when_tracking` and itests `UC-079` (`uc_079.py::the_mouse_reaches_an_application_that_asked_for_it_and_no_other`) and `UC-061` (`uc_061.py::vim_keeps_its_screen_modes_and_buffer_across_a_reload_and_writes_it`).

#### Scenario: A program turns on SGR mouse
- **WHEN** the program prints `ESC[?1000h ESC[?1006h`
- **THEN** the next frame has `mouse_tracking` true

### Requirement: Terminal queries are answered by the pane
Replies the terminal owes the program (device status and cursor position reports, device attributes and the like) SHALL be written back to the pty by the pane itself, whether or not a client watches.

**Verified by**: unit test `ghostty::tests::replies_to_queries`.

#### Scenario: Cursor position report
- **WHEN** the program at the top-left cell prints `ESC[6n`
- **THEN** it reads `ESC[1;1R` from its terminal

### Requirement: Scrolling the view through history
`Scroll{delta}` SHALL move the pane's view by `delta` rows, negative into history, stopping at the first line of history and at the live screen; `Scroll{to_bottom: true}` SHALL return to the live screen, ignoring `delta`. The view SHALL always show consecutive lines.

**Verified by**: `scroll::scrolling_back_shows_history_and_scrolling_forward_returns`, `scroll::to_bottom_returns_to_the_live_screen` and `scroll::scrolling_stops_at_both_ends_of_history`.

#### Scenario: Back and forward
- **WHEN** a 30-row pane printed `line-1` .. `line-200` and a client scrolls by -50
- **THEN** the screen shows `line-140` (with `line-139` and `line-141`) but not `line-200`, and scrolling by +50 shows `line-200` again

#### Scenario: Past both ends
- **WHEN** a client scrolls by -1000000 twice and then by +5
- **THEN** the view shows `line-1` after the first and `line-30` after the +5, and +1000000 returns to `line-200`

### Requirement: viewport_top says where the view is
`Frame.viewport_top` SHALL be the number of history rows above the viewport's first row: 0 with no history, growing by one per line that scrolls off while the view is at the live screen, and fixed while the view is scrolled back (new output does not move it).

**Verified by**: `scroll::scrolling_back_shows_history_and_scrolling_forward_returns` and unit test `ghostty::tests::scrollback_viewport`.

#### Scenario: Scrolled up by 50
- **WHEN** a client at the live screen with `viewport_top` T scrolls by -50
- **THEN** the frame's `viewport_top` is T - 50, and T again after scrolling by +50

### Requirement: The view belongs to the pane
The scroll position SHALL be the pane's, shared by every viewer: a scroll by one client SHALL move the view of all, and a client subscribing to a scrolled pane SHALL see the scrolled view. Output arriving while the view is scrolled back SHALL NOT move it.

**Verified by**: `scroll::the_scroll_position_is_shared_by_everyone_watching`, `scroll::a_client_joining_a_scrolled_pane_sees_the_scrolled_view` and `scroll::output_arriving_while_scrolled_back_does_not_move_the_view`.

#### Scenario: Another viewer scrolls
- **WHEN** one of two viewers scrolls by -100
- **THEN** the other viewer's screen shows `line-90` and not `line-200`

#### Scenario: Output while reading history
- **WHEN** the view is scrolled back and the program prints `new output`
- **THEN** the view still shows the same history lines and `new output` shows only after `Scroll{to_bottom}`

### Requirement: History stays with the pane
A pane's history and scroll position SHALL survive its clients reconnecting and the server reloading (see `reload`).

**Verified by**: `scroll::history_is_still_there_after_reconnecting`, `scroll::history_survives_a_reload` and `scroll::a_scrolled_view_stays_scrolled_through_a_reload`.

#### Scenario: After a reconnect
- **WHEN** a client reconnects to a pane that printed 200 lines and scrolls by -1000000
- **THEN** it sees `line-1`

#### Scenario: Scrolled through a reload
- **WHEN** the view is scrolled back to `line-90` and the server reloads
- **THEN** a client subscribing after the reload sees `line-90` and not `line-200`

### Requirement: Typing returns the view to the live screen
`KeyInput` and `Paste` SHALL return the pane's view to the live screen before they are written; `MouseInput` and `RawInput` SHALL NOT move it.

**Verified by**: `scroll::typing_returns_the_view_to_the_live_screen`.

#### Scenario: A key while scrolled back
- **WHEN** the view is scrolled back by 100 and a client presses `x`
- **THEN** the screen shows `line-200` again

### Requirement: Keys are encoded as the terminal's modes say
`KeyInput` SHALL name the key by `named` or by `codepoint` (the unshifted character), with `text`, `mods` (1 shift, 2 alt, 4 ctrl, 8 super) and `action` (press, release, repeat). The pane SHALL write what an xterm-compatible terminal sends for it in the program's modes (application cursor keys, the kitty keyboard protocol, Alt as an ESC prefix).

**Verified by**: unit tests `ghostty::tests::encodes_keys`, `ghostty::tests::arrow_keys_follow_application_mode` and itests `UC-079` (`uc_079.py::ctrl_alt_function_and_cursor_keys_reach_the_program_as_xterm_sends_them`).

#### Scenario: Basic keys
- **WHEN** a client sends `a`, Shift+`a` with text `A`, Ctrl+`c`, Alt+`a`, Enter and Up
- **THEN** the program reads `a`, `A`, 0x03, `ESC a`, `\r` and `ESC[A`

#### Scenario: Application cursor mode
- **WHEN** the program printed `ESC[?1h` and a client presses Up
- **THEN** the program reads `ESC O A`

#### Scenario: Function and editing keys
- **WHEN** a client sends Ctrl-A..Z, Alt keys, F1..F12, the arrows and the editing keys to a program logging its input (`uc_079.py::ctrl_alt_function_and_cursor_keys_reach_the_program_as_xterm_sends_them`)
- **THEN** the program reads xterm's bytes for each

### Requirement: Text and Shift of a key
Shift that produced the key's `text` SHALL be consumed, control characters in `text` SHALL be ignored (the control code comes from the key and its mods), and a key that cannot be encoded SHALL be dropped without harming the pane. Input SHALL be appended after input still waiting for the pty.

**Verified by**: unit tests `ghostty::tests::encodes_keys` and `ghostty::tests::encoding_appends_to_a_nearly_full_buffer`.

#### Scenario: Shift+a
- **WHEN** a client sends `codepoint` 'a', `mods` shift, `text` `A`
- **THEN** the program reads `A`

#### Scenario: Ctrl+Shift+F5 after a click
- **WHEN** a mouse click and Ctrl+Shift+F5 are encoded after five bytes still waiting
- **THEN** the pty gets the five bytes, then `ESC[<0;3;2M`, then the key's sequence

### Requirement: Ctrl and Alt go by the physical key on a non-Latin layout
`KeyInput.code` SHALL be the physical key, in W3C `KeyboardEvent.code` names (`KeyA`..`KeyZ`, `Digit0`..`Digit9`, `Backquote`, `Backslash`, `BracketLeft`, `BracketRight`, `Comma`, `Equal`, `Minus`, `Period`, `Quote`, `Semicolon`, `Slash`, `Space`); with Ctrl or Alt and a non-ASCII character (`codepoint`, else `text`) the key SHALL be encoded as its US-layout character.

**Verified by**: unit test `ghostty::tests::ctrl_and_alt_on_a_layout_that_is_not_latin_go_by_the_physical_key` and itests `UC-079` (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`).

#### Scenario: Ctrl+C on a Russian layout
- **WHEN** a client sends `KeyInput{code: "KeyC", codepoint: 'с', mods: ctrl}`
- **THEN** the program reads 0x03 (and `sleep` in the foreground is interrupted)

#### Scenario: More Russian keys
- **WHEN** a client sends Ctrl+Я (`KeyZ`), Alt+Ф (`KeyA`), Alt+Shift+Ф and Ctrl+Ъ (`BracketRight`)
- **THEN** the program reads 0x1A, `ESC a`, `ESC A` and 0x1D (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`)

#### Scenario: Typing on a Russian layout
- **WHEN** a client sends `с` (`KeyC`) without Ctrl or Alt, or Shift+`с` with text `С`
- **THEN** the program reads `с` or `С`

### Requirement: An ASCII character wins over the physical key
When the key's character is ASCII it SHALL be encoded as itself wherever its key is (Dvorak, AZERTY).

**Verified by**: unit test `ghostty::tests::a_latin_character_wins_over_the_physical_key` and itests `UC-079` (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`).

#### Scenario: Dvorak
- **WHEN** a client sends `KeyInput{code: "KeyI", codepoint: 'c', mods: ctrl}`
- **THEN** the program reads 0x03

#### Scenario: AZERTY
- **WHEN** a client sends Ctrl+`a` with `code` `KeyQ`, or Ctrl+`w` with `code` `KeyZ`
- **THEN** the program reads 0x01 or 0x17

### Requirement: The kitty keyboard protocol gets the physical key
While the program has the kitty keyboard protocol on, the physical key SHALL be reported as the base layout key, alongside the character.

**Verified by**: unit test `ghostty::tests::the_kitty_protocol_gets_the_physical_key_as_the_base_layout_key`.

#### Scenario: Ctrl+с under kitty
- **WHEN** the program printed `ESC[>5u` and a client sends Ctrl+`с` with `code` `KeyC`
- **THEN** the program reads `ESC[1089::99;5u`

#### Scenario: Dvorak under kitty
- **WHEN** a client sends Ctrl+`c` with `code` `KeyI`
- **THEN** the program reads `ESC[99::105;5u`

### Requirement: A key without a physical key is taken as on a US layout
A `KeyInput` with `code` empty (an older client) SHALL be encoded from `codepoint` as if the layout were US; Ctrl with a character that is not ASCII SHALL then send nothing.

**Verified by**: unit test `ghostty::tests::ctrl_and_alt_on_a_layout_that_is_not_latin_go_by_the_physical_key`.

#### Scenario: An older client without code
- **WHEN** a client sends Ctrl with `codepoint` 'с' and `code` empty
- **THEN** the program reads nothing

### Requirement: Mouse input reaches only a program that asked for it
`MouseInput{action, button, mods, x, y}` (zero-based cells) SHALL be encoded in the mouse protocol the program turned on, and SHALL write nothing while it has not asked for mouse events. In SGR mode a press or motion SHALL be `ESC[<b;x+1;y+1M` and a release `...m`, with wheel up/down as 64/65, a held button in motion adding 32 and Ctrl adding 16.

**Verified by**: unit test `ghostty::tests::mouse_encoded_only_when_tracking` and itests `UC-079` (`uc_079.py::the_mouse_reaches_an_application_that_asked_for_it_and_no_other`).

#### Scenario: No tracking
- **WHEN** a client clicks in a pane whose program did not ask for the mouse
- **THEN** the program reads nothing

#### Scenario: SGR click
- **WHEN** the program printed `ESC[?1000h ESC[?1006h` and a client presses the left button at x 2, y 1
- **THEN** the program reads `ESC[<0;3;2M`

#### Scenario: Drag, wheel and Ctrl
- **WHEN** a program with SGR mouse on gets a left drag to x 12, y 5, a wheel up at (7, 7) and a Ctrl+left press at (1, 1) (`uc_079.py::the_mouse_reaches_an_application_that_asked_for_it_and_no_other`)
- **THEN** it reads `ESC[<32;13;6M`, `ESC[<64;8;8M` and `ESC[<16;2;2M`

### Requirement: Pasting
`Paste{text}` SHALL be written as the text alone while the program has not turned on bracketed paste, and between `ESC[200~` and `ESC[201~` while it has (`ESC[?2004h`); an end marker inside the text SHALL NOT end the bracket early.

**Verified by**: unit test `ghostty::tests::paste_respects_bracketed_mode` and itests `UC-079` (`uc_079.py::bracketed_paste_wraps_the_text_and_cannot_be_ended_by_the_text`, `uc_079.py::a_paste_without_bracketed_mode_is_plain_text`).

#### Scenario: Bracketed
- **WHEN** the program printed `ESC[?2004h` and a client pastes `hi`
- **THEN** the program reads `ESC[200~hi ESC[201~`

#### Scenario: A paste that tries to end the bracket
- **WHEN** a client pastes `harmless ESC[201~rm -rf ~` into a bracketed-paste program
- **THEN** the program reads one `ESC[201~`, at the end

### Requirement: Raw input and input order
`RawInput{data}` SHALL be written to the pty as it is. All input of a pane (keys, mouse, paste, raw, and the terminal's own replies) SHALL be written in the order it arrived; input the pty cannot take yet SHALL wait, not be dropped.

**Verified by**: `metrics::a_frame_names_the_input_it_shows_and_how_it_was_made` and `metrics::a_scroll_is_credited_while_key_input_waits_for_the_pty`.

#### Scenario: A line as raw bytes
- **WHEN** a client sends `RawInput` with `echo ready\n`
- **THEN** the shell runs it and prints `ready`

### Requirement: Kitty graphics placements
Kitty graphics images in view, virtual placements through U+10EEEE placeholder cells (sent as blank cells) included, SHALL be sent as `Frame.placements` of the viewport, with `placements_changed` set and the complete list on every full frame and whenever the placements in view changed; unset SHALL mean the last list holds.

**Verified by**: unit tests `ghostty::tests::placements_and_images_are_sent_only_when_they_change`, `ghostty::tests::placeholder_cells_show_their_part_of_a_virtual_placement`.

#### Scenario: An image after two letters
- **WHEN** after `ab` the program sends a 20 x 4 image with id 7
- **THEN** the full frame has one placement of image 7 at col 2, row 0, covering 10 x 2 cells, 100 x 40 pixels, and the cursor at (12, 1)

#### Scenario: Text under an image
- **WHEN** the program prints text on a row below a shown image
- **THEN** the frame carries that row with `placements_changed` false, no placements and no images

#### Scenario: Scrolled by one row
- **WHEN** the image's top row scrolls off the screen
- **THEN** the frame has `placements_changed` set, the placement's `row` is -1, and no image data is sent again

#### Scenario: Deleted
- **WHEN** the program deletes the image in view
- **THEN** the frame has `placements_changed` set, no placements and no rows

### Requirement: What a placement says
A `Placement` SHALL give the image's `image_id` and `image_generation`, the top-left `col`/`row` (row negative when the top is scrolled out), the `cols`/`rows` covered, the source rectangle in image pixels, the pixel offset in its cell, `z` (negative under the text) and `pixel_width`/`pixel_height` in the frame's cell.

**Verified by**: unit test `ghostty::tests::kitty_image_comes_as_a_placement_and_a_png`.

#### Scenario: Source size
- **WHEN** the program shows a whole 20 x 4 image
- **THEN** the placement's `source_width` and `source_height` are 20 and 4

### Requirement: Kitty graphics images
Images SHALL be sent as PNG in `Frame.images` (`id`, `generation`, `width`, `height`, `png`): each (id, generation) once to the pane's viewers in the frame where it first shows, and every image in view again on a full frame. Sending an image again under the same id SHALL give it a higher `generation`. A pane SHALL hold at most 320 MiB of image data.

**Verified by**: unit tests `ghostty::tests::kitty_image_comes_as_a_placement_and_a_png`, `ghostty::tests::an_image_sent_again_gets_a_new_generation`.

#### Scenario: Re-sent image
- **WHEN** the program sends image 7 again
- **THEN** the next frame carries image 7 with a higher `generation`, and its placement names that generation

### Requirement: The cell size in pixels
A pane's cell SHALL be 10 x 20 pixels until a `Resize` gives `cell_width_px` and `cell_height_px` (both non-zero); 0 SHALL keep the current cell. Placements SHALL be measured in that cell, programs asking for the screen size in pixels SHALL get cells times that size, and every frame SHALL carry it in `cell_width_px`/`cell_height_px`.

**Verified by**: unit test `ghostty::tests::placements_are_measured_in_the_client_cell`.

#### Scenario: A client with 8 x 16 cells
- **WHEN** a pane's cell is set to 8 x 16 and the program shows a 20 x 4 image
- **THEN** frames carry `cell_width_px` 8 and `cell_height_px` 16, and the placement covers 10 x 2 cells at 80 x 32 pixels

## Notes

- Requirements without a test: none at requirement level. Parts without a test: the 8 ms coalescing, the 320 MiB image limit, the dropping of a key that cannot be encoded, the cursor shapes and blinking.
- Unit tests `ghostty::tests::*` are in crates/tilr-screen/src/ghostty.rs; the screen model is tested there, end to end only through `UC-079` and the cargo tests in `scroll.rs`/`panes.rs`.
- USE_CASES.md "Не покрыто" lists scrolling while the pane is resized as untested; nothing here specifies it.
- UC-079 was red until 2026-10-17 (Ctrl+С on a Russian layout sent nothing) and is green at HEAD per itests/USE_CASES.md.
