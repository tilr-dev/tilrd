//! What the driver knows of a pane's screen: its rows as plain text, kept up
//! to date from the frames.

use serde_json::{Value, json};
use tilr_proto::Frame;

#[derive(Default)]
pub struct Screen {
    pub rows: Vec<String>,
    pub cols: u32,
    pub cursor: (u32, u32),
    pub cursor_visible: bool,
    pub alt_screen: bool,
    pub title: String,
    pub frames: u64,
    pub last_seq: u64,
    /// No full frame since the last subscribe: the rows are not the pane's.
    pub stale: bool,
}

impl Screen {
    pub fn fresh() -> Self {
        Self {
            stale: true,
            ..Self::default()
        }
    }

    pub fn apply(&mut self, frame: &Frame) {
        if frame.full {
            self.rows = vec![String::new(); frame.rows as usize];
            self.stale = false;
        } else if self.rows.len() != frame.rows as usize {
            self.rows.resize(frame.rows as usize, String::new());
        }
        self.cols = frame.cols;
        for row in &frame.changed {
            if let Some(slot) = self.rows.get_mut(row.y as usize) {
                *slot = row.runs.iter().map(|run| run.text.as_str()).collect();
            }
        }
        if let Some(cursor) = &frame.cursor {
            self.cursor = (cursor.x, cursor.y);
            self.cursor_visible = cursor.visible;
        }
        if let Some(title) = &frame.title {
            self.title.clone_from(title);
        }
        self.alt_screen = frame.alt_screen;
        self.frames += 1;
        self.last_seq = frame.seq;
    }

    pub fn text(&self) -> String {
        let rows: Vec<&str> = self.rows.iter().map(|row| row.trim_end()).collect();
        rows.join("\n")
    }

    pub fn to_json(&self) -> Value {
        let rows: Vec<&str> = self.rows.iter().map(|row| row.trim_end()).collect();
        json!({
            "rows": rows,
            "text": self.text(),
            "cols": self.cols,
            "cursor": [self.cursor.0, self.cursor.1],
            "cursor_visible": self.cursor_visible,
            "alt_screen": self.alt_screen,
            "title": self.title,
            "frames": self.frames,
            "frame_seq": self.last_seq,
        })
    }
}
