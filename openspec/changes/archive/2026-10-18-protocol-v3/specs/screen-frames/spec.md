## REMOVED Requirements

### Requirement: A key without a physical key is taken as on a US layout
**Reason**: Its scenario called an empty `code` "an older client"; in protocol 3 every client knows `code`, so an empty one means a key with no physical position. Replaced by "A key without a physical key is encoded as on a US layout" with the same behavior.
**Migration**: None: the encoding is unchanged.

## ADDED Requirements

### Requirement: A key without a physical key is encoded as on a US layout
A `KeyInput` with `code` empty (a key the client cannot place on a physical key: synthetic input from a script, or a character key of `tilr`, which reads keys from its own terminal as characters) SHALL be encoded from `codepoint` as if the layout were US; Ctrl with a character that is not ASCII SHALL then send nothing.

**Verified by**: unit test `ghostty::tests::a_key_without_a_physical_key_is_encoded_as_on_a_us_layout`.

#### Scenario: Synthetic input without code
- **WHEN** a client sends Ctrl with `codepoint` 'с' and `code` empty
- **THEN** the program reads nothing

#### Scenario: A Latin key without code
- **WHEN** a client sends Ctrl with `codepoint` 'c' and `code` empty
- **THEN** the program reads 0x03
