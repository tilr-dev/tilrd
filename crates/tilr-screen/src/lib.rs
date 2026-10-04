//! Terminal screen model behind a backend-neutral trait.
//!
//! The rest of tilr talks to [`Screen`] only, so the VT engine (currently
//! libghostty-vt) can be swapped or vendored without touching the server.

mod ghostty;

use anyhow::Result;
use tilr_proto::{Frame, KeyInput, MouseInput, PaneStatus};

pub use ghostty::GhosttyScreen;

pub trait Screen {
    /// Feed bytes read from the pty.
    fn feed(&mut self, bytes: &[u8]);

    /// Bytes the terminal wants written back to the pty (replies to queries
    /// such as device attributes or cursor position reports).
    fn take_replies(&mut self) -> Vec<u8>;

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;

    fn size(&self) -> (u16, u16);

    /// Move the viewport by `delta` rows; negative scrolls into history.
    fn scroll(&mut self, delta: i32);

    fn scroll_to_bottom(&mut self);

    /// Build the next frame. With `full` every row is included; otherwise only
    /// rows changed since the previous call, and `None` if nothing changed.
    /// `pane_id` and `seq` are left for the caller to fill in.
    fn frame(&mut self, full: bool) -> Result<Option<Frame>>;

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
