//! Key names as tests write them: `enter`, `esc`, `up`, `f5`, `ctrl-c`,
//! `alt-x`, `shift-tab`, `q`; `ctrl-с@KeyC` names the physical key too
//! (`KeyInput.code`).
//!
//! With a keyboard layout (`us`, `ru`, `dvorak`, `azerty`) a character key
//! is sent as a client on that layout sends it: the physical key the
//! character is on, and the character as its text even with Ctrl.

use anyhow::{Result, bail};
use tilr_proto::{KeyInput, NamedKey, mods};

/// The physical keys that type characters, as W3C KeyboardEvent.code names
/// them, in the order of [`US`] and the other layouts' rows.
const CODES: [&str; 47] = [
    "Backquote",
    "Digit1",
    "Digit2",
    "Digit3",
    "Digit4",
    "Digit5",
    "Digit6",
    "Digit7",
    "Digit8",
    "Digit9",
    "Digit0",
    "Minus",
    "Equal",
    "KeyQ",
    "KeyW",
    "KeyE",
    "KeyR",
    "KeyT",
    "KeyY",
    "KeyU",
    "KeyI",
    "KeyO",
    "KeyP",
    "BracketLeft",
    "BracketRight",
    "Backslash",
    "KeyA",
    "KeyS",
    "KeyD",
    "KeyF",
    "KeyG",
    "KeyH",
    "KeyJ",
    "KeyK",
    "KeyL",
    "Semicolon",
    "Quote",
    "KeyZ",
    "KeyX",
    "KeyC",
    "KeyV",
    "KeyB",
    "KeyN",
    "KeyM",
    "Comma",
    "Period",
    "Slash",
];
/// What each of [`CODES`] types, unshifted, on a layout.
const US: &str = "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./";
const RU: &str = "ё1234567890-=йцукенгшщзхъ\\фывапролджэячсмитьбю.";
const DVORAK: &str = "`1234567890[]',.pyfgcrl/=\\aoeuidhtns-;qjkxbmwvz";
const AZERTY: &str = "²&é\"'(-è_çà)=azertyuiop^$*qsdfghjklmùwxcvbn,;:!";

/// The physical key `ch` is on in `layout`.
fn code_of(layout: &str, ch: char) -> Result<&'static str> {
    let row = match layout {
        "us" => US,
        "ru" => RU,
        "dvorak" => DVORAK,
        "azerty" => AZERTY,
        other => bail!("unknown layout {other:?} (us, ru, dvorak, azerty)"),
    };
    let ch = ch.to_lowercase().next().unwrap_or(ch);
    match row.chars().position(|c| c == ch) {
        Some(at) => Ok(CODES[at]),
        None => bail!("{ch:?} is not on the {layout} layout"),
    }
}

/// `spec` as a key of pane `pane_id`, typed on `layout` if given.
pub fn parse(pane_id: &str, spec: &str, layout: Option<&str>) -> Result<KeyInput> {
    let (spec, code) = match spec.rsplit_once('@') {
        Some((spec, code)) if !spec.is_empty() => (spec, Some(code)),
        _ => (spec, None),
    };
    let mut mods_mask = 0;
    let mut rest = spec;
    loop {
        let lower = rest.to_ascii_lowercase();
        let (bit, len) = if lower.starts_with("ctrl-") {
            (mods::CTRL, 5)
        } else if lower.starts_with("alt-") {
            (mods::ALT, 4)
        } else if lower.starts_with("shift-") {
            (mods::SHIFT, 6)
        } else if lower.starts_with("super-") {
            (mods::SUPER, 6)
        } else {
            break;
        };
        mods_mask |= bit;
        rest = &rest[len..];
    }
    let mut key = KeyInput {
        pane_id: pane_id.to_owned(),
        mods: mods_mask,
        ..Default::default()
    };
    key.code = code.unwrap_or_default().to_owned();
    if let Some(named) = named(&rest.to_ascii_lowercase()) {
        key.named = named as i32;
        return Ok(key);
    }
    let mut chars = rest.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        bail!("unknown key: {spec:?}");
    };
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    key.codepoint = u32::from(lower);
    let shifted = || {
        if mods_mask & mods::SHIFT != 0 {
            ch.to_uppercase().to_string()
        } else {
            ch.to_string()
        }
    };
    if let Some(layout) = layout {
        if code.is_none() {
            key.code = code_of(layout, ch)?.to_owned();
        }
        // A client on a layout says which character the key has there, with
        // Ctrl too (a browser's KeyboardEvent.key).
        if mods_mask & mods::SUPER == 0 {
            key.text = shifted();
        }
        return Ok(key);
    }
    // The text the key types: with Alt (as Option-as-Alt) too, so Alt+Shift+X is "X";
    // Ctrl and Super make none.
    if mods_mask & (mods::CTRL | mods::SUPER) == 0 {
        key.text = shifted();
    }
    Ok(key)
}

fn named(name: &str) -> Option<NamedKey> {
    Some(match name {
        "enter" | "return" => NamedKey::Enter,
        "tab" => NamedKey::Tab,
        "backspace" | "bs" => NamedKey::Backspace,
        "esc" | "escape" => NamedKey::Escape,
        "space" => NamedKey::Space,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" | "pgup" => NamedKey::PageUp,
        "pagedown" | "pgdn" => NamedKey::PageDown,
        "insert" => NamedKey::Insert,
        "delete" | "del" => NamedKey::Delete,
        "f1" => NamedKey::F1,
        "f2" => NamedKey::F2,
        "f3" => NamedKey::F3,
        "f4" => NamedKey::F4,
        "f5" => NamedKey::F5,
        "f6" => NamedKey::F6,
        "f7" => NamedKey::F7,
        "f8" => NamedKey::F8,
        "f9" => NamedKey::F9,
        "f10" => NamedKey::F10,
        "f11" => NamedKey::F11,
        "f12" => NamedKey::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_names_the_physical_key_and_the_text() {
        let key = parse("p", "ctrl-с", Some("ru")).unwrap();
        assert_eq!(
            (
                key.mods,
                key.codepoint,
                key.text.as_str(),
                key.code.as_str()
            ),
            (mods::CTRL, 'с' as u32, "с", "KeyC")
        );
        assert_eq!(parse("p", "ctrl-я", Some("ru")).unwrap().code, "KeyZ");
        assert_eq!(parse("p", "alt-shift-ф", Some("ru")).unwrap().text, "Ф");
        assert_eq!(parse("p", "ctrl-c", Some("dvorak")).unwrap().code, "KeyI");
        assert_eq!(parse("p", "ctrl-a", Some("azerty")).unwrap().code, "KeyQ");
        assert_eq!(parse("p", "ctrl-c@KeyX", Some("us")).unwrap().code, "KeyX");
        assert_eq!(parse("p", "ctrl-с@KeyC", None).unwrap().code, "KeyC");
        assert!(parse("p", "ctrl-я", Some("us")).is_err());
    }

    #[test]
    fn ctrl_c_is_the_c_key_with_ctrl_and_no_text() {
        let key = parse("p", "ctrl-c", None).unwrap();
        assert_eq!(
            (key.mods, key.codepoint, key.text.as_str()),
            (mods::CTRL, 'c' as u32, "")
        );
    }

    #[test]
    fn named_keys_and_plain_characters_are_understood() {
        assert_eq!(
            parse("p", "Enter", None).unwrap().named,
            NamedKey::Enter as i32
        );
        assert_eq!(parse("p", "shift-tab", None).unwrap().mods, mods::SHIFT);
        assert_eq!(parse("p", "q", None).unwrap().text, "q");
        let upper = parse("p", "G", None).unwrap();
        assert_eq!((upper.codepoint, upper.text.as_str()), ('g' as u32, "G"));
        assert!(parse("p", "nonsense", None).is_err());
    }
}
