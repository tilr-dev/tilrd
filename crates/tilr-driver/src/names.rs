//! File names that are not UTF-8, in JSON lines.
//!
//! A name on Linux is any bytes but '/' and NUL; JSON strings are Unicode.
//! The driver writes such a name as Python's `os.fsdecode` does
//! (surrogateescape): each byte that is not part of valid UTF-8 becomes the
//! lone surrogate U+DC80 + byte, `\udcXX` in JSON. A test reads it with
//! `json.loads`, joins it to a path, and sends it back with `json.dumps`
//! (`\udcXX` again), and the driver turns it back into the same bytes: what
//! goes to the server as `path_bytes`.
//!
//! Rust strings cannot hold a lone surrogate, so inside the driver each such
//! byte is a character of the last private use plane, U+10FF80 + (byte -
//! 0x80): [`decode_line`] makes them of the `\udcXX` in a command, and
//! [`encode_line`] makes them `\udcXX` again in an event.

use std::fmt::Write;

/// Where the stand-ins of bytes 0x80..=0xFF start.
const BASE: u32 = 0x10FF80;

fn stand_in(byte: u8) -> char {
    char::from_u32(BASE + u32::from(byte - 0x80)).expect("a valid char")
}

fn byte_of(ch: char) -> Option<u8> {
    let code = u32::from(ch);
    (BASE..=BASE + 0x7F)
        .contains(&code)
        .then(|| (code - BASE + 0x80) as u8)
}

/// A command line with every lone `\udc80`..`\udcff` escape made the
/// character that stands in for its byte, so that it parses.
pub fn decode_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.char_indices().peekable();
    // The last escape was a high surrogate: the next one is its pair.
    let mut high = false;
    while let Some((at, ch)) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            high = false;
            continue;
        }
        let Some((_, next)) = chars.next() else {
            out.push(ch);
            break;
        };
        let escape = (next == 'u')
            .then(|| line.get(at + 2..at + 6))
            .flatten()
            .and_then(|hex| u16::from_str_radix(hex, 16).ok());
        match escape {
            Some(unit @ 0xDC80..=0xDCFF) if !high => {
                out.push(stand_in((unit & 0xFF) as u8));
                for _ in 0..4 {
                    chars.next();
                }
                high = false;
            }
            Some(unit) => {
                out.push_str(&line[at..at + 6]);
                for _ in 0..4 {
                    chars.next();
                }
                high = (0xD800..=0xDBFF).contains(&unit);
            }
            None => {
                out.push(ch);
                out.push(next);
                high = false;
            }
        }
    }
    out
}

/// An event line with every stand-in written as its `\udcXX` escape.
pub fn encode_line(line: &str) -> String {
    if !line.chars().any(|ch| byte_of(ch).is_some()) {
        return line.to_owned();
    }
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        match byte_of(ch) {
            Some(byte) => {
                let _ = write!(out, "\\udc{byte:02x}");
            }
            None => out.push(ch),
        }
    }
    out
}

/// A path from a command, for a request: the string, and the raw bytes too
/// when it has stand-ins (the string then has U+FFFD for them).
pub fn to_request(path: &str) -> (String, Vec<u8>) {
    if !path.chars().any(|ch| byte_of(ch).is_some()) {
        return (path.to_owned(), Vec::new());
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut shown = String::with_capacity(path.len());
    for ch in path.chars() {
        match byte_of(ch) {
            Some(byte) => {
                bytes.push(byte);
                shown.push(char::REPLACEMENT_CHARACTER);
            }
            None => {
                bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                shown.push(ch);
            }
        }
    }
    (shown, bytes)
}

/// A name or path from an answer, for an event: the string, or, when the
/// server gave the raw bytes (not UTF-8), the bytes with stand-ins.
pub fn from_answer(name: &str, bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return name.to_owned();
    }
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        out.extend(chunk.invalid().iter().map(|&byte| stand_in(byte.max(0x80))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_not_utf8_goes_out_and_comes_back_as_its_bytes() {
        let raw = b"/w/bin\xff\xfe-\xc3(name.dat";
        let event = serde_json::json!({ "name": from_answer("shown", raw) }).to_string();
        let line = encode_line(&event);
        assert_eq!(line, r#"{"name":"/w/bin\udcff\udcfe-\udcc3(name.dat"}"#);
        let back: serde_json::Value = serde_json::from_str(&decode_line(&line)).unwrap();
        let (shown, bytes) = to_request(back["name"].as_str().unwrap());
        assert_eq!(bytes, raw);
        assert_eq!(shown, "/w/bin\u{fffd}\u{fffd}-\u{fffd}(name.dat");
    }

    #[test]
    fn other_escapes_and_names_are_left_as_they_are() {
        // An emoji as Python's json.dumps writes it: a surrogate pair whose
        // low half is in the range of the stand-ins.
        let line = r#"{"path":"🂀 \\udcff é \n"}"#;
        assert_eq!(decode_line(line), line);
        let value: serde_json::Value = serde_json::from_str(&decode_line(line)).unwrap();
        assert_eq!(
            to_request(value["path"].as_str().unwrap()),
            ("\u{1f080} \\udcff é \n".to_owned(), Vec::new())
        );
        assert_eq!(from_answer("plain", b""), "plain");
    }
}
