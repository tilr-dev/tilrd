## ADDED Requirements

### Requirement: Hyperlinks go with their runs
A run of cells the program printed inside an OSC 8 hyperlink SHALL carry `Run.link`, the index plus one of its URI in the frame's `links`, which SHALL list each URI the frame's rows use once; runs SHALL split where the link changes, and cells without a link SHALL have `link` 0. A URI longer than 8 KiB SHALL NOT be listed and its cells SHALL go without a link.

**Verified by**: unit tests `ghostty::tests::a_hyperlink_goes_with_its_run` and `ghostty::tests::an_overlong_hyperlink_is_left_out` (crates/tilr-screen/src/ghostty.rs).

#### Scenario: A link between plain text
- **WHEN** the program prints `a ESC]8;;https://example.com/ESC\ link ESC]8;;ESC\ b`
- **THEN** row 0 has a run `link` with `link` 1 between runs with `link` 0, and the frame's `links` is `["https://example.com/"]`

#### Scenario: Two rows, one link
- **WHEN** the same URI is used on rows 0 and 1 of one frame
- **THEN** `links` lists it once and both runs point at it

#### Scenario: No links
- **WHEN** a frame's rows have no hyperlinks
- **THEN** its `links` is empty and every `Run.link` is 0

### Requirement: Hyperlinks on the screen do not survive a reload
After a reload, cells that had a hyperlink before it SHALL be sent as plain text with the same characters and styles; output after the reload SHALL carry its links as usual.

**Verified by**: `reload::hyperlinks_are_plain_text_after_a_reload_and_new_ones_work`.

#### Scenario: A link printed before a reload
- **WHEN** a pane shows a link, the server reloads and the client reconnects
- **THEN** the full frame shows the link's text with `link` 0, and a link printed afterwards has `link` 1
