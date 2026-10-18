//! Attached to a pane: the terminal shows its frames and sends it the keys,
//! the mouse (when the program in the pane asks for it), pastes and the
//! terminal's size.
//!
//! The terminal is in raw mode on the alternate screen. Ctrl+] is the
//! prefix: Ctrl+] d detaches, Ctrl+] Ctrl+] sends Ctrl+] itself; any other
//! key after it is sent as usual.
//!
//! A connection that drops (the network, the server reloading) is made
//! again with the same client id; meanwhile the bottom row says so, and
//! what is typed is dropped, not sent later. A local server that is gone for
//! good (its socket nobody's) ends the attach.

use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as TermButton,
    MouseEvent, MouseEventKind,
};
use crossterm::style::{
    Attribute, Color as TermColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
    SetUnderlineColor,
};
use crossterm::terminal::{
    BeginSynchronizedUpdate, Clear, ClearType, DisableLineWrap, EnableLineWrap,
    EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute, queue, terminal};
use futures_util::StreamExt;
use tilr_proto::{
    Color, CursorShape, Frame, KeyAction, KeyInput, MouseAction, MouseButton, MouseInput, NamedKey,
    Paste, Resize, Scroll, Style, Subscribe, Underline, attrs, client_message::Msg, color, mods,
    server_message,
};
use tokio::time::Instant;

use crate::conn::{Conn, ConnectError};
use crate::daemon;
use crate::target::Target;

const WHEEL_SCROLL_ROWS: i32 = 3;
/// How often a connection is pinged, and how long it may stay silent.
const PING_EVERY: Duration = Duration::from_secs(5);
const SILENT_FOR: Duration = Duration::from_secs(15);
/// Waits between reconnect attempts: from the first to the last, doubling.
const RETRY_FIRST: Duration = Duration::from_millis(100);
const RETRY_LAST: Duration = Duration::from_secs(2);

/// How an attach ended.
pub enum Exit {
    Detached,
    PaneExited(i32),
    /// The pane was closed elsewhere, or was gone after a reconnect.
    PaneGone,
    /// The local server stopped.
    ServerGone,
}

/// Attach to `pane_id` over `conn` until detached or the pane ends.
pub async fn run(conn: Conn, target: &Target, client_id: &str, pane_id: &str) -> Result<Exit> {
    let guard = TerminalGuard::enter()?;
    let mut attach = Attach {
        target,
        client_id,
        pane_id,
        mouse_captured: false,
        prefix: false,
    };
    let outcome = attach.event_loop(conn).await;
    drop(guard);
    outcome
}

struct Attach<'a> {
    target: &'a Target,
    client_id: &'a str,
    pane_id: &'a str,
    /// The outer terminal reports the mouse: the pane's program wants it.
    mouse_captured: bool,
    /// Ctrl+] was pressed: the next key is a command.
    prefix: bool,
}

/// What one key does.
enum Keyed {
    Send(Msg),
    Detach,
    Nothing,
}

impl Attach<'_> {
    async fn event_loop(&mut self, conn: Conn) -> Result<Exit> {
        let mut input = EventStream::new();
        let mut conn = Some(conn);
        if let Some(conn) = &mut conn {
            self.subscribe(conn).await?;
        }
        let mut ping = tokio::time::interval(PING_EVERY);
        let mut retry = RETRY_FIRST;
        let mut attempts = 0u32;
        let mut next_attempt = Instant::now();
        loop {
            let Some(live) = &mut conn else {
                // Disconnected: keys other than the prefix's are dropped.
                tokio::select! {
                    event = input.next() => {
                        let Some(event) = event else { return Ok(Exit::Detached) };
                        if let Event::Key(key) = event?
                            && let Keyed::Detach = self.key(&key)
                        {
                            return Ok(Exit::Detached);
                        }
                    }
                    () = tokio::time::sleep_until(next_attempt) => {
                        attempts += 1;
                        match Conn::connect(self.target, self.client_id).await {
                            Ok(mut fresh) => {
                                if !fresh.list_panes().await?.iter().any(|pane| pane.pane_id == self.pane_id) {
                                    return Ok(Exit::PaneGone);
                                }
                                self.subscribe(&mut fresh).await?;
                                conn = Some(fresh);
                                retry = RETRY_FIRST;
                                attempts = 0;
                            }
                            Err(ConnectError::NoServer(_))
                                if matches!(self.target, Target::Local(path) if daemon::stopped(path)) =>
                            {
                                return Ok(Exit::ServerGone);
                            }
                            // Another user's socket now: nothing to wait for.
                            Err(ConnectError::Untrusted(err)) => return Err(err),
                            Err(err) => {
                                status(&format!("tilr: disconnected ({err}), reconnecting… (attempt {attempts}; Ctrl+] d detaches)"))?;
                                next_attempt = Instant::now() + retry;
                                retry = (retry * 2).min(RETRY_LAST);
                            }
                        }
                    }
                }
                continue;
            };
            let lost = tokio::select! {
                msg = live.recv() => match msg {
                    Ok(server_message::Msg::Frame(frame)) if frame.pane_id == self.pane_id => {
                        self.capture_mouse(frame.mouse_tracking)?;
                        draw(&frame)?;
                        None
                    }
                    Ok(server_message::Msg::PaneExited(exited)) if exited.pane_id == self.pane_id => {
                        return Ok(Exit::PaneExited(exited.exit_code));
                    }
                    Ok(server_message::Msg::PaneList(list))
                        if !list.panes.iter().any(|pane| pane.pane_id == self.pane_id) =>
                    {
                        return Ok(Exit::PaneGone);
                    }
                    Ok(_) => None,
                    Err(err) => Some(err.to_string()),
                },
                event = input.next() => {
                    let Some(event) = event else { return Ok(Exit::Detached) };
                    let msg = match event? {
                        Event::Key(key) => match self.key(&key) {
                            Keyed::Send(msg) => Some(msg),
                            Keyed::Detach => return Ok(Exit::Detached),
                            Keyed::Nothing => None,
                        },
                        Event::Mouse(mouse) => mouse_message(self.pane_id, &mouse, self.mouse_captured),
                        Event::Paste(text) => Some(Msg::Paste(Paste {
                            pane_id: self.pane_id.to_owned(),
                            text,
                        })),
                        Event::Resize(cols, rows) if cols > 0 && rows > 0 => {
                            Some(resize(self.pane_id, cols, rows))
                        }
                        _ => None,
                    };
                    match msg {
                        Some(msg) => live.send(msg).await.err().map(|err| err.to_string()),
                        None => None,
                    }
                }
                _ = ping.tick() => {
                    if live.heard.elapsed() > SILENT_FOR {
                        Some(format!("nothing heard for {}s", SILENT_FOR.as_secs()))
                    } else {
                        live.ping().await.err().map(|err| err.to_string())
                    }
                }
            };
            if let Some(why) = lost {
                conn = None;
                status(&format!(
                    "tilr: disconnected ({why}), reconnecting… (Ctrl+] d detaches)"
                ))?;
                next_attempt = Instant::now() + RETRY_FIRST;
            }
        }
    }

    /// Size the pane to this terminal and watch it; the server answers with
    /// a full frame.
    async fn subscribe(&self, conn: &mut Conn) -> Result<()> {
        let (cols, rows) = crate::size();
        conn.send(resize(self.pane_id, cols, rows)).await?;
        conn.send(Msg::Subscribe(Subscribe {
            pane_id: self.pane_id.to_owned(),
        }))
        .await
    }

    fn capture_mouse(&mut self, wanted: bool) -> Result<()> {
        if wanted != self.mouse_captured {
            if wanted {
                execute!(std::io::stdout(), EnableMouseCapture)?;
            } else {
                execute!(std::io::stdout(), DisableMouseCapture)?;
            }
            self.mouse_captured = wanted;
        }
        Ok(())
    }

    /// A key: the prefix and its commands, or input for the pane.
    fn key(&mut self, key: &KeyEvent) -> Keyed {
        if key.kind == KeyEventKind::Release {
            return Keyed::Nothing;
        }
        if std::mem::take(&mut self.prefix) {
            if is_prefix(key) {
                return key_message(self.pane_id, key).map_or(Keyed::Nothing, Keyed::Send);
            }
            if key.code == KeyCode::Char('d')
                && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
            {
                return Keyed::Detach;
            }
        } else if is_prefix(key) {
            self.prefix = true;
            return Keyed::Nothing;
        }
        key_message(self.pane_id, key).map_or(Keyed::Nothing, Keyed::Send)
    }
}

/// Ctrl+]. Terminals without the kitty keyboard protocol send it as the
/// control code 0x1D, which crossterm reads as Ctrl+5.
fn is_prefix(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char(']' | '5'))
}

fn resize(pane_id: &str, cols: u16, rows: u16) -> Msg {
    Msg::Resize(Resize {
        pane_id: pane_id.to_owned(),
        cols: u32::from(cols),
        rows: u32::from(rows),
        ..Default::default()
    })
}

/// A line on the bottom row, over the pane's screen, until the next frame
/// draws that row.
fn status(text: &str) -> Result<()> {
    let (cols, rows) = crate::size();
    let text: String = text.chars().take(usize::from(cols)).collect();
    let mut out = std::io::stdout().lock();
    queue!(
        out,
        cursor::SavePosition,
        cursor::MoveTo(0, rows.saturating_sub(1)),
        SetAttribute(Attribute::Reset),
        SetAttribute(Attribute::Reverse),
        Print(text),
        Clear(ClearType::UntilNewLine),
        SetAttribute(Attribute::Reset),
        cursor::RestorePosition,
    )?;
    out.flush()?;
    Ok(())
}

/// Restores the terminal on drop, including on error paths.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            DisableLineWrap,
            EnableBracketedPaste,
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            EnableLineWrap,
            SetAttribute(Attribute::Reset),
            cursor::SetCursorStyle::DefaultUserShape,
            cursor::Show,
            LeaveAlternateScreen,
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn draw(frame: &Frame) -> Result<()> {
    let mut out = std::io::stdout().lock();
    queue!(out, BeginSynchronizedUpdate, cursor::Hide)?;
    if frame.full {
        queue!(out, SetAttribute(Attribute::Reset), Clear(ClearType::All))?;
    }
    for row in &frame.changed {
        queue!(out, cursor::MoveTo(0, row.y as u16))?;
        for run in &row.runs {
            let style = run
                .style
                .checked_sub(1)
                .and_then(|index| frame.styles.get(index as usize));
            apply_style(&mut out, style)?;
            queue!(out, Print(&run.text))?;
        }
        queue!(
            out,
            SetAttribute(Attribute::Reset),
            Clear(ClearType::UntilNewLine)
        )?;
    }
    if let Some(cursor) = &frame.cursor
        && cursor.visible
    {
        use cursor::SetCursorStyle as Shape;
        let shape = match (cursor.shape(), cursor.blinking) {
            (CursorShape::Bar, true) => Shape::BlinkingBar,
            (CursorShape::Bar, false) => Shape::SteadyBar,
            (CursorShape::Underline, true) => Shape::BlinkingUnderScore,
            (CursorShape::Underline, false) => Shape::SteadyUnderScore,
            (_, true) => Shape::BlinkingBlock,
            (_, false) => Shape::SteadyBlock,
        };
        queue!(
            out,
            cursor::MoveTo(cursor.x as u16, cursor.y as u16),
            shape,
            cursor::Show
        )?;
    }
    queue!(out, EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

fn apply_style(out: &mut impl Write, style: Option<&Style>) -> Result<()> {
    queue!(out, SetAttribute(Attribute::Reset))?;
    let Some(style) = style else { return Ok(()) };

    if let Some(color) = term_color(style.fg.as_ref()) {
        queue!(out, SetForegroundColor(color))?;
    }
    if let Some(color) = term_color(style.bg.as_ref()) {
        queue!(out, SetBackgroundColor(color))?;
    }
    for (bit, attribute) in [
        (attrs::BOLD, Attribute::Bold),
        (attrs::ITALIC, Attribute::Italic),
        (attrs::FAINT, Attribute::Dim),
        (attrs::BLINK, Attribute::SlowBlink),
        (attrs::INVERSE, Attribute::Reverse),
        (attrs::INVISIBLE, Attribute::Hidden),
        (attrs::STRIKETHROUGH, Attribute::CrossedOut),
        (attrs::OVERLINE, Attribute::OverLined),
    ] {
        if style.attrs & bit != 0 {
            queue!(out, SetAttribute(attribute))?;
        }
    }
    let underline = match style.underline() {
        Underline::None => None,
        Underline::Single => Some(Attribute::Underlined),
        Underline::Double => Some(Attribute::DoubleUnderlined),
        Underline::Curly => Some(Attribute::Undercurled),
        Underline::Dotted => Some(Attribute::Underdotted),
        Underline::Dashed => Some(Attribute::Underdashed),
    };
    if let Some(attribute) = underline {
        queue!(out, SetAttribute(attribute))?;
        if let Some(color) = term_color(style.underline_color.as_ref()) {
            queue!(out, SetUnderlineColor(color))?;
        }
    }
    Ok(())
}

fn term_color(color: Option<&Color>) -> Option<TermColor> {
    match color?.value? {
        color::Value::Palette(index) => Some(TermColor::AnsiValue(index as u8)),
        color::Value::Rgb(rgb) => Some(TermColor::Rgb {
            r: (rgb >> 16) as u8,
            g: (rgb >> 8) as u8,
            b: rgb as u8,
        }),
    }
}

fn proto_mods(modifiers: KeyModifiers) -> u32 {
    let mut out = 0;
    for (modifier, bit) in [
        (KeyModifiers::SHIFT, mods::SHIFT),
        (KeyModifiers::ALT, mods::ALT),
        (KeyModifiers::CONTROL, mods::CTRL),
        (KeyModifiers::SUPER, mods::SUPER),
    ] {
        if modifiers.contains(modifier) {
            out |= bit;
        }
    }
    out
}

/// The key as `KeyInput`. The physical key (`code`) is sent for the keys a
/// terminal names (Enter, the arrows, F1…): those are the same key on any
/// layout. A character key arrives as the character the layout made, with
/// Ctrl folded in by the terminal (Ctrl+С on a Russian layout comes as ^C,
/// read as Ctrl+c), so its physical key is not known and `code` stays empty.
fn key_message(pane_id: &str, key: &KeyEvent) -> Option<Msg> {
    let mut mods = proto_mods(key.modifiers);
    let mut codepoint = 0;
    let mut text = String::new();
    let (named, code) = match key.code {
        KeyCode::Char(' ') => {
            text.push(' ');
            (NamedKey::Space, "Space")
        }
        KeyCode::Char(ch) => {
            // The produced character; the key itself is its lowercase form
            // for letters.
            codepoint = ch.to_lowercase().next().unwrap_or(ch) as u32;
            if ch.is_uppercase() {
                mods |= mods::SHIFT;
            }
            // 0x1C..0x1F come as Ctrl+4..7: they are Ctrl+\ ] ^ _.
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && let Some(real) = match ch {
                    '4' => Some('\\'),
                    '5' => Some(']'),
                    '6' => Some('^'),
                    '7' => Some('_'),
                    _ => None,
                }
            {
                codepoint = real as u32;
            }
            text.push(ch);
            (NamedKey::Unspecified, "")
        }
        KeyCode::Enter => (NamedKey::Enter, "Enter"),
        KeyCode::Tab => (NamedKey::Tab, "Tab"),
        KeyCode::BackTab => {
            mods |= mods::SHIFT;
            (NamedKey::Tab, "Tab")
        }
        KeyCode::Backspace => (NamedKey::Backspace, "Backspace"),
        KeyCode::Esc => (NamedKey::Escape, "Escape"),
        KeyCode::Up => (NamedKey::Up, "ArrowUp"),
        KeyCode::Down => (NamedKey::Down, "ArrowDown"),
        KeyCode::Left => (NamedKey::Left, "ArrowLeft"),
        KeyCode::Right => (NamedKey::Right, "ArrowRight"),
        KeyCode::Home => (NamedKey::Home, "Home"),
        KeyCode::End => (NamedKey::End, "End"),
        KeyCode::PageUp => (NamedKey::PageUp, "PageUp"),
        KeyCode::PageDown => (NamedKey::PageDown, "PageDown"),
        KeyCode::Insert => (NamedKey::Insert, "Insert"),
        KeyCode::Delete => (NamedKey::Delete, "Delete"),
        KeyCode::F(n @ 1..=12) => (
            NamedKey::try_from(NamedKey::F1 as i32 + i32::from(n) - 1).ok()?,
            F_CODES[usize::from(n) - 1],
        ),
        _ => return None,
    };
    // With Ctrl held the character is a key name, not text to insert.
    if mods & mods::CTRL != 0 {
        text.clear();
    }
    Some(Msg::Key(KeyInput {
        pane_id: pane_id.to_owned(),
        action: match key.kind {
            KeyEventKind::Press => KeyAction::Press,
            KeyEventKind::Repeat => KeyAction::Repeat,
            KeyEventKind::Release => KeyAction::Release,
        } as i32,
        mods,
        named: named as i32,
        codepoint,
        text,
        code: code.to_owned(),
    }))
}

const F_CODES: [&str; 12] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

fn mouse_message(pane_id: &str, mouse: &MouseEvent, tracking: bool) -> Option<Msg> {
    let button = |button: TermButton| match button {
        TermButton::Left => MouseButton::Left,
        TermButton::Right => MouseButton::Right,
        TermButton::Middle => MouseButton::Middle,
    };
    let (action, button) = match mouse.kind {
        MouseEventKind::Down(b) => (MouseAction::Press, button(b)),
        MouseEventKind::Up(b) => (MouseAction::Release, button(b)),
        MouseEventKind::Drag(b) => (MouseAction::Motion, button(b)),
        MouseEventKind::Moved => (MouseAction::Motion, MouseButton::None),
        MouseEventKind::ScrollUp => (MouseAction::Press, MouseButton::WheelUp),
        MouseEventKind::ScrollDown => (MouseAction::Press, MouseButton::WheelDown),
        _ => return None,
    };
    if tracking {
        return Some(Msg::Mouse(MouseInput {
            pane_id: pane_id.to_owned(),
            action: action as i32,
            button: button as i32,
            mods: proto_mods(mouse.modifiers),
            x: u32::from(mouse.column),
            y: u32::from(mouse.row),
        }));
    }
    // The application does not want the mouse: the wheel scrolls history.
    let delta = match button {
        MouseButton::WheelUp => -WHEEL_SCROLL_ROWS,
        MouseButton::WheelDown => WHEEL_SCROLL_ROWS,
        _ => return None,
    };
    Some(Msg::Scroll(Scroll {
        pane_id: pane_id.to_owned(),
        delta,
        to_bottom: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn attach() -> Attach<'static> {
        static TARGET: std::sync::LazyLock<Target> =
            std::sync::LazyLock::new(|| Target::Local("/nowhere".into()));
        Attach {
            target: &TARGET,
            client_id: "c",
            pane_id: "p",
            mouse_captured: false,
            prefix: false,
        }
    }

    #[test]
    fn ctrl_bracket_d_detaches_and_twice_sends_it() {
        let mut a = attach();
        let ctrl = KeyModifiers::CONTROL;
        assert!(matches!(
            a.key(&press(KeyCode::Char('5'), ctrl)),
            Keyed::Nothing
        ));
        assert!(matches!(
            a.key(&press(KeyCode::Char('d'), KeyModifiers::NONE)),
            Keyed::Detach
        ));

        assert!(matches!(
            a.key(&press(KeyCode::Char(']'), ctrl)),
            Keyed::Nothing
        ));
        let Keyed::Send(Msg::Key(key)) = a.key(&press(KeyCode::Char('5'), ctrl)) else {
            panic!("Ctrl+] twice sends one");
        };
        assert_eq!((key.codepoint, key.mods), (']' as u32, mods::CTRL));

        // Anything else after the prefix is just sent.
        assert!(matches!(
            a.key(&press(KeyCode::Char('5'), ctrl)),
            Keyed::Nothing
        ));
        let Keyed::Send(Msg::Key(key)) = a.key(&press(KeyCode::Char('x'), KeyModifiers::NONE))
        else {
            panic!("x is sent");
        };
        assert_eq!(key.text, "x");
        assert!(matches!(
            a.key(&press(KeyCode::Char('d'), KeyModifiers::NONE)),
            Keyed::Send(_)
        ));
    }

    #[test]
    fn named_keys_carry_their_physical_key() {
        let Some(Msg::Key(key)) = key_message("p", &press(KeyCode::Up, KeyModifiers::NONE)) else {
            panic!();
        };
        assert_eq!(key.code, "ArrowUp");
        let Some(Msg::Key(key)) = key_message("p", &press(KeyCode::F(5), KeyModifiers::NONE))
        else {
            panic!();
        };
        assert_eq!((key.named, key.code.as_str()), (NamedKey::F5 as i32, "F5"));
        let Some(Msg::Key(key)) =
            key_message("p", &press(KeyCode::Char('c'), KeyModifiers::CONTROL))
        else {
            panic!();
        };
        assert_eq!(
            (key.codepoint, key.code.as_str(), key.text.as_str()),
            ('c' as u32, "", "")
        );
    }
}
