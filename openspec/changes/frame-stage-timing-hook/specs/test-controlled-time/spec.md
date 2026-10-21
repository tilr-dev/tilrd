## ADDED Requirements

### Requirement: Test-controlled frame stage durations
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the pane SHALL support `TILR_TEST_FRAME_STAGE_TIME_FILE` assigning exact parse, build and encode durations by advancing only its measurement clock. It SHALL preserve parse accumulation and previous-frame encode reporting. Without this setting, existing timing SHALL remain unchanged. The hook SHALL be compiled out without `failpoints`.

**Verified by**: `metrics::frame_stage_durations_are_controlled_per_operation`.

#### Scenario: Distinct operation durations
- **GIVEN** the stage file contains three whitespace-separated positive decimal microsecond durations in parse, build, encode order
- **WHEN** a test fixes the shared timestamp and sets the stage durations to distinct values
- **THEN** each emitted frame reports parse equal to the configured parse duration multiplied by its output read count and build equal to the configured build duration
- **AND** its encode value equals the duration configured for the previous emitted frame, initially zero
- **AND** scheduling and operational deadlines retain their existing clocks

#### Scenario: Configuration changes between frames
- **WHEN** the test atomically replaces the stage file after receiving a frame and requests another frame from a pane with no pending output
- **THEN** the next frame reports the new build duration and the old encode duration
- **AND** the following frame reports the new encode duration
