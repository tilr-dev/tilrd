## ADDED Requirements

### Requirement: Hyperlinks go with their runs
Frames SHALL list each retained OSC 8 URI once in `links`; `Run.link` SHALL be its index plus one, or 0 for plain text. Runs SHALL split where the link changes. URIs exceeding the VT engine's OSC payload limit or the defensive 8 KiB frame buffer, or containing invalid UTF-8, SHALL be omitted and their cells SHALL remain plain text.

**Verified by**: unit tests `ghostty::tests::a_hyperlink_goes_with_its_run`, `ghostty::tests::an_overlong_hyperlink_is_left_out`, `ghostty::tests::the_vt_osc_limit_keeps_text_but_drops_long_links`, `ghostty::tests::an_out_of_space_uri_is_left_out`, `ghostty::tests::a_non_utf8_hyperlink_is_plain_text` and `ghostty::tests::linked_trailing_spaces_keep_their_cells` (crates/tilr-screen/src/ghostty.rs).

#### Scenario: A link between plain text
- **WHEN** the program prints `a ESC]8;;https://example.com/ESC\ link ESC]8;;ESC\ b`
- **THEN** row 0 has a run `link` with `link` 1 between runs with `link` 0, and the frame's `links` is `["https://example.com/"]`

#### Scenario: Two rows, one link
- **WHEN** the same URI is used on rows 0 and 1 of one frame
- **THEN** `links` lists it once and both runs point at it

#### Scenario: The VT engine rejects a long OSC URI
With libghostty-vt 0.2.2, checked 2026-10-24, a parameter-free OSC 8 retains a 2046-byte URI and drops a 2047-byte URI. OSC parameters also consume the engine's payload budget. Recheck with `cargo test -p tilr-screen the_vt_osc_limit_keeps_text_but_drops_long_links --locked`.

- **WHEN** an OSC 8 URI exceeds the VT engine's OSC payload limit
- **THEN** the label remains plain text with link 0 and the URI is absent from `links`

#### Scenario: The URI read exceeds the defensive buffer
- **WHEN** reading a cell URI returns `OutOfSpace` for the 8 KiB URI buffer
- **THEN** the frame retains the label as plain text without listing the URI

#### Scenario: No links
- **WHEN** a frame's rows have no hyperlinks
- **THEN** its `links` is empty and every `Run.link` is 0

#### Scenario: Invalid UTF-8 in a URI
- **WHEN** the program prints a hyperlink URI containing invalid UTF-8
- **THEN** the frame still carries the text, with no hyperlink

#### Scenario: Linked trailing spaces retain the hyperlink
- **WHEN** an OSC 8 label ends with spaces or consists entirely of spaces
- **THEN** its linked cells remain in the frame, while unlinked trailing blank cells may be omitted

### Requirement: Attach draws hyperlinks in the local terminal
The attach client SHALL surround each linked run's text with OSC 8 opening and closing sequences using its URI from the frame table. Unlinked runs SHALL be drawn as plain text.

**Verified by**: `attach::tests::a_linked_run_is_drawn_inside_osc_8`.

#### Scenario: A linked run followed by plain text
- **WHEN** attach draws a linked run followed by an unlinked run
- **THEN** it opens OSC 8 with the URI before the linked text and closes it before the plain text

### Requirement: Hyperlinks on the screen do not survive a reload
After a reload, cells that had a hyperlink before it SHALL be sent as plain text with the same characters and styles; output after the reload SHALL carry its links as usual.

**Verified by**: `reload::hyperlinks_are_plain_text_after_a_reload_and_new_ones_work`.

#### Scenario: A link printed before a reload
- **WHEN** a pane shows a link, the server reloads and the client reconnects
- **THEN** the full frame shows the link's text with `link` 0, and a link printed afterwards has `link` 1
