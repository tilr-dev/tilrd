## MODIFIED Requirements

### Requirement: A key without a physical key is taken as on a US layout
A `KeyInput` with `code` empty (a key the client cannot place on a physical key, such as synthetic input from a script) SHALL be encoded from `codepoint` as if the layout were US; Ctrl with a character that is not ASCII SHALL then send nothing.

**Verified by**: unit test `ghostty::tests::ctrl_and_alt_on_a_layout_that_is_not_latin_go_by_the_physical_key`.

#### Scenario: An older client without code
- **WHEN** a client (or a script) sends Ctrl with `codepoint` 'с' and `code` empty
- **THEN** the program reads nothing
