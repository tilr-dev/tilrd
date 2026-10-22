## ADDED Requirements

### Requirement: Fresh test shells are ready before input

Shared test helpers that open and subscribe to an interactive shell SHALL wait
for its first prompt before returning. Tests that deliberately send input before
startup SHALL retain explicit control of that ordering.

**Verified by**: `reconnect::opening_a_fresh_pane_waits_for_the_first_shell_prompt`, `reconnect::reconnect_output_when_input_precedes_the_shell_prompt`.

#### Scenario: Shell startup is held
- **WHEN** a test holds the shell before its first prompt and opens a pane through the shared helper
- **THEN** opening remains pending until startup is released and the prompt is observed
- **AND** the first command's output can be matched as an exact row without adding a leading newline

#### Scenario: A test deliberately precedes startup
- **WHEN** a test explicitly uses the controlled startup fixture
- **THEN** it can observe PTY input echo before releasing the shell's first prompt
