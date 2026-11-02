use tilr_proto::{Frame, color};
use tilr_screen::{GhosttyScreen, Screen};

fn text(frame: &Frame, y: u32) -> String {
    frame
        .changed
        .iter()
        .find(|row| row.y == y)
        .unwrap()
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect()
}

#[test]
fn erase_preserves_split_csi_and_rendition() {
    for history in [false, true] {
        let mut screen = GhosttyScreen::new(20, 5).unwrap();
        screen.feed(b"before\x1b[31");
        screen.erase(history);
        assert!(screen.take_replies().is_empty());
        screen.feed(b"mX");
        let frame = screen.frame(true).unwrap().unwrap();
        assert_eq!(text(&frame, 0), "X");
        let run = &frame.changed[0].runs[0];
        assert_eq!(
            frame.styles[run.style as usize - 1]
                .fg
                .as_ref()
                .unwrap()
                .value,
            Some(color::Value::Palette(1))
        );
    }
}

#[test]
fn erase_preserves_split_osc() {
    for history in [false, true] {
        let mut screen = GhosttyScreen::new(20, 5).unwrap();
        screen.feed(b"before\x1b]2;unfinished");
        screen.erase(history);
        assert!(screen.take_replies().is_empty());
        screen.feed(b" title\x1b\\X");
        assert_eq!(screen.take_status().unwrap().title, "unfinished title");
        assert_eq!(text(&screen.frame(true).unwrap().unwrap(), 0), "X");
    }
}

#[test]
fn erase_preserves_split_dcs() {
    for history in [false, true] {
        let mut screen = GhosttyScreen::new(20, 5).unwrap();
        // DCS payload stays inside the string until ST, including after erase.
        screen.feed(b"\x1b[31m\x1bP$q");
        screen.erase(history);
        assert!(screen.take_replies().is_empty());
        screen.feed(b"m\x1b\\Y");
        assert!(screen.take_replies().is_empty());
        assert_eq!(text(&screen.frame(true).unwrap().unwrap(), 0), "Y");
    }
}

#[test]
fn erase_homes_absolutely_and_preserves_origin_margins_and_modes() {
    for history in [false, true] {
        let mut screen = GhosttyScreen::new(20, 5).unwrap();
        screen.feed(b"\x1b[2;4r\x1b[?6h\x1b[?2004h\x1b[?1h\x1b[31mold");
        screen.erase(history);
        screen.feed(b"X");
        assert_eq!(text(&screen.frame(true).unwrap().unwrap(), 0), "X");
        screen.feed(b"\x1b[1;1HY\x1b[6n");
        assert_eq!(screen.take_replies(), b"\x1b[1;2R");
        assert_eq!(text(&screen.frame(true).unwrap().unwrap(), 1), "Y");
        let mut encoded = Vec::new();
        screen.encode_paste("paste", &mut encoded).unwrap();
        assert_eq!(encoded, b"\x1b[200~paste\x1b[201~");
        screen.feed(b"\x1b[3;1HZ\nW");
        let frame = screen.frame(true).unwrap().unwrap();
        assert_eq!(text(&frame, 0), "X");
        assert_eq!(text(&frame, 2), "Z");
        assert_eq!(text(&frame, 3), " W");
    }
}

#[test]
fn erase_applies_history_policy_on_primary_and_alternate_screens() {
    for history in [false, true] {
        let mut screen = GhosttyScreen::new(20, 3).unwrap();
        screen.feed(b"history\r\nline\r\nother\r\nactive");
        screen.erase(history);
        let frame = screen.frame(true).unwrap().unwrap();
        assert!(frame.changed.iter().all(|row| row.runs.is_empty()));
        screen.scroll(-100);
        let frame = screen.frame(true).unwrap().unwrap();
        assert_eq!(screen.scrolled_back() > 0, !history);
        if !history {
            assert_eq!(text(&frame, 0), "history");
        }
        screen.scroll_to_bottom();
        screen.feed(b"primary\x1b[?1049halternate");
        screen.erase(history);
        screen.feed(b"X");
        let frame = screen.frame(true).unwrap().unwrap();
        assert!(frame.alt_screen);
        assert_eq!(text(&frame, 0), "X");
        screen.feed(b"\x1b[?1049l");
        assert_eq!(text(&screen.frame(true).unwrap().unwrap(), 0), "primary");
    }
}
