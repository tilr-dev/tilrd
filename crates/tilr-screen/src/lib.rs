//! Terminal screen model behind a backend-neutral trait.
//!
//! The rest of tilr talks to [`Screen`] only, so the VT engine (currently
//! libghostty-vt) can be swapped or vendored without touching the server.

mod ghostty;

use anyhow::Result;
use tilr_proto::{Frame, KeyInput, MouseInput, PaneStatus};

pub use ghostty::{DEFAULT_CELL_PX, GhosttyScreen};

pub trait Screen {
    /// Feed bytes read from the pty.
    fn feed(&mut self, bytes: &[u8]);

    /// Bytes the terminal wants written back to the pty (replies to queries
    /// such as device attributes or cursor position reports).
    fn take_replies(&mut self) -> Vec<u8>;

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
