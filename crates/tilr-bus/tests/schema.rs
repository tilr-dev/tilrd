//! The payload schemas keep the field numbers of `tilr.v1` (design T1).
//!
//! The fixtures `tests/fixtures/tilr_v1_*.bin` are `tilr.v1` messages, encoded from the
//! `.txt` next to them with the schema of `crates/tilr-proto/proto/tilr.proto`:
//!
//! ```sh
//! protoc -I crates/tilr-proto/proto --encode=tilr.v1.Frame tilr.proto \
//!   < tilr_v1_frame.txt > tilr_v1_frame.bin
//! protoc -I crates/tilr-proto/proto --encode=tilr.v1.PaneList tilr.proto \
//!   < tilr_v1_pane_list.txt > tilr_v1_pane_list.bin
//! protoc -I crates/tilr-proto/proto --encode=tilr.v1.ClientMessage tilr.proto \
//!   < tilr_v1_key_input.txt > tilr_v1_key_input.bin
//! ```
//!
//! They stay when `tilr.proto` is removed.

use prost::Message;
use tilr_bus::pane::{
    self, Color, CursorShape, Frame, InputRequest, KeyAction, ListReply, Underline, color,
    input_request,
};

#[test]
fn a_tilr_v1_frame_decodes_as_a_pane_frame() {
    let bytes = include_bytes!("fixtures/tilr_v1_frame.bin");
    let frame = Frame::decode(&bytes[..]).unwrap();
    assert_eq!(frame.links, ["https://example.com/"]);
    assert_eq!(frame.pane_id, "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4");
    assert_eq!(
        (frame.seq, frame.full, frame.cols, frame.rows),
        (42, true, 80, 2)
    );
    let style = &frame.styles[0];
    assert_eq!(
        style.fg,
        Some(Color {
            value: Some(color::Value::Palette(1))
        })
    );
    assert_eq!(
        style.bg,
        Some(Color {
            value: Some(color::Value::Rgb(0x102030))
        })
    );
    assert_eq!(style.attrs, pane::attrs::BOLD | pane::attrs::FAINT);
    assert_eq!(style.underline(), Underline::Curly);
    let runs = &frame.changed[0].runs;
    assert_eq!(
        (runs[0].style, runs[0].text.as_str(), runs[0].link),
        (1, "hello", 1)
    );
    assert_eq!((runs[1].text.as_str(), runs[1].wide), ("世", true));
    assert_eq!(frame.changed[1].y, 1);
    assert!(frame.changed[1].runs[0].cluster);
    let cursor = frame.cursor.unwrap();
    assert_eq!((cursor.visible, cursor.x, cursor.y), (true, 3, 1));
    assert_eq!(cursor.shape(), CursorShape::Bar);
    assert_eq!(frame.title.as_deref(), Some("vim"));
    assert!(frame.mouse_tracking && frame.alt_screen && frame.placements_changed);
    let placement = &frame.placements[0];
    assert_eq!(
        (placement.image_id, placement.row, placement.z),
        (7, -1, -1)
    );
    assert_eq!(frame.images[0].png.as_ref(), b"\x89PNG");
    assert_eq!((frame.cell_width_px, frame.cell_height_px), (10, 20));
    assert_eq!(frame.viewport_top, 100);
    assert_eq!(frame.last_input_seq, (1 << 32) + 1);
    let timing = frame.timing.unwrap();
    assert_eq!((timing.wait_us, timing.input_us), (1, u32::MAX));
    // Nothing is lost: encoded again, the bytes are the same.
    assert_eq!(frame.encode_to_vec(), bytes);
}

#[test]
fn a_tilr_v1_pane_list_decodes_as_the_reply_of_pane_list() {
    let bytes = include_bytes!("fixtures/tilr_v1_pane_list.bin");
    let reply = ListReply::decode(&bytes[..]).unwrap();
    let info = &reply.panes[0];
    assert_eq!(info.pane_id, "p-1");
    assert_eq!((info.cols, info.rows), (100, 30));
    assert_eq!(info.command, ["/bin/sh"]);
    assert_eq!((info.title.as_str(), info.cwd.as_str()), ("t", "/home"));
    assert_eq!((info.viewers, info.created_at_ms, info.pid), (1, 1000, 77));
    assert_eq!(info.viewed_by[0].client_name, "tilr");
    assert_eq!(info.viewed_by[0].address, "127.0.0.1:5");
    assert_eq!(info.used_at_ms, 2000);
    // New in the bus: empty in a tilr.v1 message.
    assert_eq!(info.created_by, "");
    assert!(!info.sandboxed);
}

#[test]
fn a_tilr_v1_key_message_decodes_as_pane_input() {
    let bytes = include_bytes!("fixtures/tilr_v1_key_input.bin");
    let request = InputRequest::decode(&bytes[..]).unwrap();
    assert_eq!((request.seq, request.input_seq), (5, 6));
    let Some(input_request::Input::Key(key)) = request.input else {
        panic!("not a key: {request:?}");
    };
    assert_eq!(key.action(), KeyAction::Press);
    assert_eq!(key.mods, pane::mods::CTRL);
    assert_eq!(
        (key.codepoint, key.text.as_str(), key.code.as_str()),
        (99, "c", "KeyC")
    );
}

#[test]
fn the_internal_input_fields_of_tilr_v1_do_not_exist_on_the_bus() {
    // ClientMessage.input_identity (102), input_probe (103), input_forget (104).
    let mut bytes = InputRequest {
        pane_id: "p".into(),
        ..Default::default()
    }
    .encode_to_vec();
    bytes.extend_from_slice(&[0xb2, 0x06, 0x01, b'x', 0xb8, 0x06, 0x01, 0xc0, 0x06, 0x01]);
    let request = InputRequest::decode(bytes.as_slice()).unwrap();
    assert_eq!(
        request.encode_to_vec(),
        InputRequest {
            pane_id: "p".into(),
            ..Default::default()
        }
        .encode_to_vec()
    );
}
