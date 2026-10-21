## ADDED Requirements

### Requirement: Test-controlled frame stage durations
In a `failpoints` build with `TILR_TEST_TIME_FILE`, a test MAY set `TILR_TEST_FRAME_STAGE_TIME_FILE` to a readable file containing three whitespace-separated positive decimal microsecond durations in parse, build, encode order. The pane SHALL advance its measurement clock inside each corresponding operation so that its measured duration equals the configured value when the shared timestamp file is held fixed. Atomic replacement of the stage file SHALL affect subsequent operations. Parse durations SHALL accumulate for the output reads represented by a frame; build SHALL describe that frame and encode SHALL describe the previous emitted frame, initially zero. Without the stage setting, existing measurement behavior SHALL remain unchanged. The hook SHALL be compiled out without `failpoints` and SHALL NOT alter scheduling or operational deadlines.

**Verified by**: `metrics::frame_stage_durations_are_controlled_per_operation`.

#### Scenario: Distinct operation durations
- **WHEN** a test fixes the shared timestamp and sets parse, build and encode durations to distinct values
- **THEN** each emitted frame reports parse equal to the configured parse duration multiplied by its output read count and build equal to the configured build duration
- **AND** its encode value equals the duration configured for the previous emitted frame

#### Scenario: Configuration changes between frames
- **WHEN** the test atomically replaces the stage file after receiving a frame and requests another frame from a pane with no pending output
- **THEN** the next frame reports the new build duration and the old encode duration
- **AND** the following frame reports the new encode duration
