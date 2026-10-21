## MODIFIED Requirements

### Requirement: Every credited input gets a frame
An input named by a seq SHALL be answered by a frame carrying its seq even when it changed nothing on the screen (a click the program ignores, an empty `RawInput`, a scroll that moves nothing). The acknowledgement SHALL include any screen changes from PTY output already read since the previous frame in the same frame, carrying the latest credited input seq. Input without a seq that changes nothing SHALL make no frame, and inputs credited between two frames SHALL show only the last of them.

**Verified by**: `metrics::input_that_changes_nothing_is_answered_with_an_empty_frame` and `metrics::a_frame_names_the_input_it_shows_and_how_it_was_made`, plus `metrics::a_no_op_scroll_frame_includes_accumulated_output`.

#### Scenario: A scroll that moves nothing
- **WHEN** a client at the live screen sends `Scroll{to_bottom: true}` with seq 8 and there are no accumulated screen changes or accumulated PTY output
- **THEN** it gets a frame with `last_input_seq` 8 and no rows

#### Scenario: Unnamed input that changes nothing
- **WHEN** a client sends input that changes nothing without a seq
- **THEN** no frame comes within 300 ms

#### Scenario: A no-op scroll with pending output
- **WHEN** PTY output has changed the screen since the previous frame and a no-op scroll with seq 8 is credited before the next frame is built
- **THEN** the frame carries `last_input_seq` 8 and the accumulated changed rows, without a separate empty acknowledgement

### Requirement: The frame for input that changed nothing
When there are no accumulated screen changes or accumulated PTY output, such a frame SHALL have `full` false, no `changed` rows, no `styles`, no `title`, no `images`, `placements_changed` false and the pane's next `seq`, with `cols`, `rows`, `cursor`, `mouse_tracking`, `alt_screen`, the cell size and `viewport_top` as in the previous frame; its `timing` SHALL have `output_bytes` and `wait_us` 0.

**Verified by**: `metrics::input_that_changes_nothing_is_answered_with_an_empty_frame`.

#### Scenario: A click without mouse tracking
- **WHEN** a client sends a `MouseInput` with seq `seq_base + 1` to a shell that does not track the mouse, with no accumulated screen changes or accumulated PTY output
- **THEN** it gets a frame with `last_input_seq` `seq_base + 1`, no rows, `seq` one more than the last frame's, and that frame's cursor, size and `viewport_top`
