//! Terminal screen model behind a backend-neutral trait.
//!
//! The rest of tilr talks to [`Screen`] only, so the VT engine (currently
//! libghostty-vt) can be swapped or vendored without touching the server.

mod ghostty;

use anyhow::Result;
use tilr_proto::{Cursor, Frame, KeyInput, MouseInput, PaneStatus};

pub use ghostty::{DEFAULT_CELL_PX, GhosttyScreen};

/// The most bytes of text [`Screen::text`] gives (4 MiB): rows of history past
/// it are left out, the oldest first; a viewport alone over it is
/// [`TextTooLarge`]. Escaped as JSON, twice this still fits a bus envelope
/// (`max_frame`, 16 MiB).
pub const MAX_TEXT_BYTES: usize = 4 << 20;

/// The screen as text, as `tmux capture-pane` gives it (`pane.read`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenText {
    /// The viewport's rows, top first, trailing blanks trimmed; a wide character
    /// once, a multi-codepoint grapheme whole.
    pub lines: Vec<String>,
    /// Rows of history right above the viewport, oldest first; fewer than
    /// asked for when [`MAX_TEXT_BYTES`] is reached.
    pub history: Vec<String>,
    /// Where the cursor is in the viewport; not visible when it is out of view.
    /// `x` is a cell column, not an index into the line's characters: a wide
    /// character takes two cells and is one character of its line.
    pub cursor: Cursor,
    pub title: String,
}

/// The viewport's text alone is over [`MAX_TEXT_BYTES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextTooLarge;

impl std::fmt::Display for TextTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the screen as text is over {MAX_TEXT_BYTES} bytes")
    }
}

impl std::error::Error for TextTooLarge {}

/// Content the application copied to the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardCopy {
    pub mime: String,
    pub data: Vec<u8>,
}

pub trait Screen {
    /// Feed bytes read from the pty.
    fn feed(&mut self, bytes: &[u8]);

    /// Erase the active screen and home to absolute (0,0), optionally deleting
    /// scrollback, preserving the program parser, modes, margins and rendition.
    /// Does not write to the PTY.
    fn erase(&mut self, clear_history: bool);

    /// Bytes the terminal wants written back to the pty (replies to queries
    /// such as device attributes or cursor position reports).
    fn take_replies(&mut self) -> Vec<u8>;

    /// Bells received since the previous call.
    fn take_bells(&mut self) -> u32;

    /// Resize to `cols` x `rows` cells of `cell` pixels each (the cell is what
    /// image placements are measured in and what pixel size reports say).
    fn resize(&mut self, cols: u16, rows: u16, cell: (u32, u32)) -> Result<()>;

    fn size(&self) -> (u16, u16);

    /// The cell in pixels of the last resize, or the default.
    fn cell_px(&self) -> (u32, u32);

    /// Move the viewport by `delta` rows; negative scrolls into history.
    fn scroll(&mut self, delta: i32);

    fn scroll_to_bottom(&mut self);

    /// How many rows the viewport is above the live screen; 0 at the bottom.
    fn scrolled_back(&self) -> u32;

    /// Build the next frame. With `full` every row is included; otherwise only
    /// rows changed since the previous call, and `None` if nothing changed.
    /// `pane_id` and `seq` are left for the caller to fill in.
    fn frame(&mut self, full: bool) -> Result<Option<Frame>>;

    /// A frame that changes nothing, for when [`Screen::frame`] has just
    /// returned `None`: the last frame's size, cursor, modes, cell and
    /// viewport, no rows, no title, placements unchanged. `None` before the
    /// first frame.
    fn unchanged_frame(&self) -> Option<Frame>;

    /// Title and working directory as reported by the application, if either
    /// changed since the previous call.
    fn take_status(&mut self) -> Option<PaneStatus>;

    /// The viewport as text with up to `history_lines` rows of history above it,
    /// at most [`MAX_TEXT_BYTES`] in all. Without the screen's scrollbar (where
    /// the viewport is in it) only the viewport is read, without history.
    /// Reading does not change what the next [`Screen::frame`] holds.
    fn text(&self, history_lines: u32) -> Result<ScreenText>;

    /// What the application copied to the standard clipboard (OSC 52, OSC
    /// 1337 Copy) since the previous call, oldest first. Other clipboards
    /// (the primary selection) are not reported.
    fn take_copies(&mut self) -> Vec<ClipboardCopy>;

    /// Encode input into the bytes the application in the pty expects, taking
    /// the terminal's current modes into account. Appends to `out`.
    fn encode_key(&mut self, key: &KeyInput, out: &mut Vec<u8>) -> Result<()>;

    fn encode_mouse(&mut self, mouse: &MouseInput, out: &mut Vec<u8>) -> Result<()>;

    fn encode_paste(&mut self, text: &str, out: &mut Vec<u8>) -> Result<()>;

    /// The screen, scrollback and terminal modes as bytes that rebuild them
    /// when fed to a fresh screen of the same size. Taking them apart uses up
    /// the screen.
    fn snapshot(self) -> Result<Vec<u8>>
    where
        Self: Sized;
}
