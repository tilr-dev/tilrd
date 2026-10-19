## MODIFIED Requirements

### Requirement: What the timing fields measure
`wait_us` SHALL be from the first pty output since the previous frame to the start of this frame's build (0 for a frame no output caused); `parse_us` feeding output into the screen model, summed; `build_us` building this frame; `encode_us` encoding and compressing the pane's previous frame (0 on its first); `output_bytes` and `reads` the pty output since the previous frame (output that changed nothing counts in the next frame) and the reads that brought it.

**Verified by**: `metrics::the_timing_fields_measure_what_they_say` (planned in change server-requirement-tests).

#### Scenario: A frame no output caused
- **WHEN** a frame answers a click that changed nothing
- **THEN** its `wait_us` and `output_bytes` are 0
