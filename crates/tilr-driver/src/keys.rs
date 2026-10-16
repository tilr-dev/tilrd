//! Key names as tests write them: `enter`, `esc`, `up`, `f5`, `ctrl-c`,
//! `alt-x`, `shift-tab`, `q`.

use anyhow::{Result, bail};
use tilr_proto::{KeyInput, NamedKey, mods};

pub fn parse(pane_id: &str, spec: &str) -> Result<KeyInput> {
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
    if let Some(named) = named(&rest.to_ascii_lowercase()) {
        key.named = named as i32;
        return Ok(key);
    }
    let mut chars = rest.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        bail!("unknown key: {spec:?}");
    };
    key.codepoint = u32::from(ch.to_ascii_lowercase());
    if mods_mask & (mods::CTRL | mods::ALT | mods::SUPER) == 0 {
        key.text = if mods_mask & mods::SHIFT != 0 {
            ch.to_ascii_uppercase().to_string()
        } else {
            ch.to_string()
        };
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
    fn ctrl_c_is_the_c_key_with_ctrl_and_no_text() {
        let key = parse("p", "ctrl-c").unwrap();
        assert_eq!(
            (key.mods, key.codepoint, key.text.as_str()),
            (mods::CTRL, 'c' as u32, "")
        );
    }

    #[test]
    fn named_keys_and_plain_characters_are_understood() {
        assert_eq!(parse("p", "Enter").unwrap().named, NamedKey::Enter as i32);
        assert_eq!(parse("p", "shift-tab").unwrap().mods, mods::SHIFT);
        assert_eq!(parse("p", "q").unwrap().text, "q");
        assert!(parse("p", "nonsense").is_err());
    }
}
