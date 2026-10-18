## RENAMED Requirements

- FROM: `### Requirement: Ctrl and Alt go by the physical key on a non-Latin layout`
- TO: `### Requirement: Ctrl goes by the physical key on a non-Latin layout`

## MODIFIED Requirements

### Requirement: Ctrl goes by the physical key on a non-Latin layout
`KeyInput.code` SHALL be the physical key, in W3C `KeyboardEvent.code` names (`KeyA`..`KeyZ`, `Digit0`..`Digit9`, `Backquote`, `Backslash`, `BracketLeft`, `BracketRight`, `Comma`, `Equal`, `Minus`, `Period`, `Quote`, `Semicolon`, `Slash`, `Space`); with Ctrl and a character that has no C0 control (`codepoint`, else `text`, not ASCII) the key SHALL be encoded as Ctrl with its US-layout character.

**Verified by**: unit test `ghostty::tests::ctrl_on_a_layout_that_is_not_latin_goes_by_the_physical_key` and itests `UC-079` (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`).

#### Scenario: Ctrl+C on a Russian layout
- **WHEN** a client sends `KeyInput{code: "KeyC", codepoint: 'с', mods: ctrl}`
- **THEN** the program reads 0x03 (and `sleep` in the foreground is interrupted)

#### Scenario: More Russian keys
- **WHEN** a client sends Ctrl+Я (`KeyZ`) and Ctrl+Ъ (`BracketRight`)
- **THEN** the program reads 0x1A and 0x1D (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`)

#### Scenario: Typing on a Russian layout
- **WHEN** a client sends `с` (`KeyC`) without Ctrl or Alt, or Shift+`с` with text `С`
- **THEN** the program reads `с` or `С`

## ADDED Requirements

### Requirement: Alt sends ESC and the character produced
With Alt and without Ctrl, a key with a character SHALL be encoded as `ESC` followed by that character (`text`, else `codepoint`, Shift applied) in UTF-8, never remapped to the physical key, whether or not `code` is given; with Ctrl and Alt it SHALL be `ESC` followed by the Ctrl encoding above. The kitty keyboard protocol, when on, SHALL be encoded by its own rules.

**Verified by**: unit test `ghostty::tests::alt_sends_esc_and_the_character_produced` and itests `UC-079` (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`).

#### Scenario: Alt on a Russian layout
- **WHEN** a client sends Alt+ж (`code` `Semicolon`) and Alt+Shift+Ф (text `Ф`, `code` `KeyA`)
- **THEN** the program reads `ESC ж` and `ESC Ф`

#### Scenario: Alt+Shift punctuation
- **WHEN** a client sends Alt+Shift+1 with text `!` on a US layout
- **THEN** the program reads `ESC !`

#### Scenario: Ctrl+Alt on a Russian layout
- **WHEN** a client sends Ctrl+Alt+с (`code` `KeyC`)
- **THEN** the program reads `ESC` 0x03

#### Scenario: Alt without a physical key
- **WHEN** a client sends Alt with text `ж` and `code` empty
- **THEN** the program reads `ESC ж`

### Requirement: Composed characters are sent as themselves
A character the client sends without Alt or Ctrl (produced by AltGr or by macOS Option composing) SHALL be sent as that character; the server SHALL treat a key as Alt only when `KeyInput.mods` says Alt.

**Verified by**: unit test `ghostty::tests::composed_characters_are_sent_as_themselves` and itests `UC-079` (`uc_079.py::ctrl_c_on_a_russian_layout_is_ctrl_c`).

#### Scenario: AltGr and Option
- **WHEN** a client sends `€` (AltGr+e), `ą` (AltGr+a) or `ç` (Option+c with Option not acting as Alt), each without Alt in `mods`
- **THEN** the program reads `€`, `ą` or `ç`

#### Scenario: Option as Alt
- **WHEN** a client whose Option acts as Alt sends Option+c as Alt with character `c`
- **THEN** the program reads `ESC c`
