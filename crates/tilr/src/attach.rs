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
//! input is buffered until acknowledged, and Ctrl+] d still detaches. A
//! local server that is gone for good (its socket nobody's) ends the attach,
//! as does a close as superseded (4000): another connection says `Hello`
//! with this client's id, and reconnecting would take the pane back from it
//! in a loop.
//!
//! Text a program in the pane copies to the clipboard (`ClipboardWrite`) is
//! written to the terminal as OSC 52, so the user's terminal puts it on the
//! clipboard of their machine, as it decides for a program over ssh.
//!
//! SIGTERM, SIGHUP and SIGINT restore the terminal, then end the process.

use std::io::Write;
use std::time::Duration;

use anyhow::Result;
use base64::Engine;
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
    ClipboardWrite, Color, CursorShape, Frame, KeyAction, KeyInput, MouseAction, MouseButton,
    MouseInput, NamedKey, Paste, Resize, Scroll, Style, Subscribe, Underline, attrs,
    client_message::Msg, color, mods, server_message,
};
use tokio::time::Instant;

use crate::conn::{Closed, Conn, ConnectError};
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
    /// The server closed the connection as superseded by a newer one of
    /// this client id.
    Superseded,
}

/// What `tilr` lists in `Hello.features` to be sent the copies made in the
/// pane it is attached to.
pub const FEATURE_CLIPBOARD_WRITE: &str = "clipboard-write";

/// Attach to `pane_id` over `conn` until detached or the pane ends.
pub async fn run(conn: Conn, target: &Target, client_id: &str, pane_id: &str) -> Result<Exit> {
    let guard = TerminalGuard::enter()?;
    let mut attach = Attach {
        target,
        client_id,
        pane_id,
        mouse_captured: false,
        prefix: false,
        copy: Incoming::default(),
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
    /// The copy coming from the pane.
    copy: Incoming,
}

/// A copy made in the pane, as its chunks come.
#[derive(Default)]
struct Incoming {
    /// The newest transfer seen; ids only grow on a connection.
    newest: u64,
    /// The copy under way: its type, size and what came of it.
    under_way: Option<(String, u64, Vec<u8>)>,
}

impl Incoming {
    /// Take a chunk. Returns what to write to the terminal once a copy of
    /// text is whole; other types are not passed on. A transfer that another
    /// one starts before its last chunk is dropped, as are late chunks of it.
    fn take(&mut self, chunk: ClipboardWrite) -> Option<Vec<u8>> {
        if chunk.transfer_id < self.newest {
            return None;
        }
        if chunk.transfer_id > self.newest {
            self.newest = chunk.transfer_id;
            self.under_way = Some((chunk.mime, chunk.total_size, Vec::new()));
        }
        let (_, total_size, data) = self.under_way.as_mut()?;
        if data.len() as u64 + chunk.data.len() as u64 > *total_size {
            // More than announced: not a copy to trust.
            self.under_way = None;
            return None;
        }
        data.extend_from_slice(&chunk.data);
        if !chunk.last {
            return None;
        }
        let (mime, total_size, data) = self.under_way.take()?;
        let text = mime
            .get(..5)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("text/"));
        (text && data.len() as u64 == total_size).then(|| osc_52(&data))
    }
}

/// The OSC 52 sequence that puts `data` on the terminal's clipboard.
fn osc_52(data: &[u8]) -> Vec<u8> {
    let mut out = b"\x1b]52;c;".to_vec();
    out.extend(
        base64::engine::general_purpose::STANDARD
            .encode(data)
            .bytes(),
    );
    out.extend(b"\x1b\\");
    out
}

/// What one key does.
enum Keyed {
    Send(Msg),
    Detach,
    Nothing,
}

impl Attach<'_> {
    fn input_event(&mut self, event: Event) -> Keyed {
        let message = match event {
            Event::Key(key) => return self.key(&key),
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
        message.map_or(Keyed::Nothing, Keyed::Send)
    }

    async fn event_loop(&mut self, conn: Conn) -> Result<Exit> {
        let mut input = EventStream::new();
        let mut delivery = tilr_proto::input::Outbox::default();
        for ack in &conn.applied_input {
            delivery.acknowledge(ack);
        }
        let mut conn = Some(conn);
        if let Some(conn) = &mut conn {
            subscribe(conn, self.pane_id).await?;
        }
        let mut ping = tokio::time::interval(PING_EVERY);
        let mut retry = RETRY_FIRST;
        let mut attempts = 0u32;
        let mut next_attempt = Instant::now();
        loop {
            let Some(live) = &mut conn else {
                // Retain input while connecting; the detach prefix remains available.
                let (target, client_id, pane_id) = (self.target, self.client_id, self.pane_id);
                let wait = next_attempt;
                let mut pending = std::pin::pin!(async move {
                    tokio::time::sleep_until(wait).await;
                    reconnect(target, client_id, pane_id).await
                });
                let outcome = loop {
                    tokio::select! {
                        outcome = &mut pending => break outcome,
                        event = input.next() => {
                            let Some(event) = event else { return Ok(Exit::Detached) };
                            match self.input_event(event?) {
                                Keyed::Detach => return Ok(Exit::Detached),
                                Keyed::Send(msg) => {
                                    if let Err(err) = delivery.push(msg.into()) { status(err)?; }
                                }
                                Keyed::Nothing => {}
                            }
                        }
                    }
                };
                attempts += 1;
                match outcome {
                    Attempt::Connected(mut fresh) => {
                        for ack in &fresh.applied_input {
                            delivery.acknowledge(ack);
                        }
                        let mut failed = false;
                        for message in delivery.pending() {
                            if let Err(err) = fresh.send_message(message.clone()).await {
                                status(&format!("tilr: replay interrupted: {err}"))?;
                                failed = true;
                                break;
                            }
                        }
                        if failed {
                            next_attempt = Instant::now() + retry;
                            continue;
                        }
                        if let Err(err) = subscribe(&mut fresh, self.pane_id).await {
                            status(&format!("tilr: disconnected ({err}), reconnecting…"))?;
                            next_attempt = Instant::now() + retry;
                            continue;
                        }
                        conn = Some(fresh);
                        // A new connection numbers its transfers afresh.
                        self.copy = Incoming::default();
                        retry = RETRY_FIRST;
                        attempts = 0;
                    }
                    Attempt::PaneGone => return Ok(Exit::PaneGone),
                    Attempt::ServerGone => return Ok(Exit::ServerGone),
                    Attempt::Superseded => return Ok(Exit::Superseded),
                    Attempt::Untrusted(err) => return Err(err),
                    Attempt::Failed(err) => {
                        status(&format!(
                            "tilr: disconnected ({err}), reconnecting… (attempt {attempts}; Ctrl+] d detaches)"
                        ))?;
                        next_attempt = Instant::now() + retry;
                        retry = (retry * 2).min(RETRY_LAST);
                    }
                }
                continue;
            };
            let lost = tokio::select! {
                msg = live.recv() => match msg {
                    Ok(server_message::Msg::InputAck(ack)) => {
                        delivery.acknowledge(&ack);
                        None
                    }
                    Ok(server_message::Msg::Frame(frame)) if frame.pane_id == self.pane_id => {
                        self.capture_mouse(frame.mouse_tracking)?;
                        draw(&frame)?;
                        None
                    }
                    Ok(server_message::Msg::ClipboardWrite(chunk)) if chunk.pane_id == self.pane_id => {
                        if let Some(osc) = self.copy.take(chunk) {
                            let mut out = std::io::stdout().lock();
                            out.write_all(&osc)?;
                            out.flush()?;
                        }
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
                    Err(err) if superseded(&err) => return Ok(Exit::Superseded),
                    Err(err) => Some(err.to_string()),
                },
                event = input.next() => {
                    let Some(event) = event else { return Ok(Exit::Detached) };
                    match self.input_event(event?) {
                        Keyed::Detach => return Ok(Exit::Detached),
                        Keyed::Send(msg) => match delivery.push(msg.into()) {
                            Ok(message) => live.send_message(message).await.err().map(|err| err.to_string()),
                            Err(err) => { status(err)?; None }
                        },
                        Keyed::Nothing => None,
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

/// How one reconnect attempt went.
enum Attempt {
    Connected(Conn),
    PaneGone,
    ServerGone,
    Superseded,
    /// Another user's socket now: nothing to wait for.
    Untrusted(anyhow::Error),
    /// Worth another attempt: the reason, for the status line.
    Failed(String),
}

/// Connect again and check the pane is still there. A
/// connection lost or a request unanswered on the way is a failed attempt,
/// as a refused connect is.
async fn reconnect(target: &Target, client_id: &str, pane_id: &str) -> Attempt {
    let mut fresh = match Conn::connect(target, client_id).await {
        Ok(fresh) => fresh,
        Err(ConnectError::NoServer(_)) if matches!(target, Target::Local(path) if daemon::stopped(path)) =>
        {
            return Attempt::ServerGone;
        }
        Err(ConnectError::Untrusted(err)) => return Attempt::Untrusted(err),
        Err(err) => return Attempt::Failed(err.to_string()),
    };
    match fresh.list_panes().await {
        Ok(panes) if !panes.iter().any(|pane| pane.pane_id == pane_id) => return Attempt::PaneGone,
        Ok(_) => {}
        Err(err) if superseded(&err) => return Attempt::Superseded,
        Err(err) => return Attempt::Failed(format!("{err:#}")),
    }
    Attempt::Connected(fresh)
}

/// The connection was closed as superseded (4000).
fn superseded(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Closed>().is_some_and(Closed::superseded)
}

/// Size the pane to this terminal and watch it; the server answers with
/// a full frame.
async fn subscribe(conn: &mut Conn, pane_id: &str) -> Result<()> {
    // Refresh after replay so an older queued Resize cannot override this size.
    // Each connection repeats this refresh, even if its previous one was lost.
    let (cols, rows) = crate::size();
    conn.send(resize(pane_id, cols, rows)).await?;
    conn.send(Msg::Subscribe(Subscribe {
        pane_id: pane_id.to_owned(),
    }))
    .await
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

/// Restores the terminal on drop, including on error paths, and on
/// SIGTERM, SIGHUP or SIGINT before the process ends by it.
struct TerminalGuard {
    /// What those signals did before.
    previous: Vec<(libc::c_int, libc::sigaction)>,
}

/// The signals that end an attached `tilr` once the terminal is restored.
const ENDING_SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGHUP, libc::SIGINT];

/// The terminal's modes before raw mode, for the signal handler.
static COOKED: std::sync::OnceLock<libc::termios> = std::sync::OnceLock::new();

/// What [`TerminalGuard`]'s drop writes, as bytes the signal handler can
/// write: the end of a frame being drawn, bracketed paste and mouse capture
/// off, line wrap on, attributes reset, the default cursor shown, the main
/// screen.
const RESTORE: &[u8] = b"\x1b[?2026l\x1b[?2004l\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?7h\x1b[0m\x1b[0 q\x1b[?25h\x1b[?1049l";

/// SIGTERM, SIGHUP or SIGINT while attached. It runs whatever the event
/// loop is doing, even blocked writing to a terminal that does not read:
/// only async-signal-safe calls, the terminal written without blocking.
extern "C" fn on_ending_signal(signal: libc::c_int) {
    // SAFETY: fcntl, write, tcsetattr, signal and raise are
    // async-signal-safe; COOKED is set before this handler is installed and
    // never changes.
    unsafe {
        let flags = libc::fcntl(1, libc::F_GETFL);
        if flags != -1 {
            libc::fcntl(1, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        libc::write(1, RESTORE.as_ptr().cast(), RESTORE.len());
        if flags != -1 {
            // The descriptor is shared with the shell that started tilr.
            libc::fcntl(1, libc::F_SETFL, flags);
        }
        if let Some(cooked) = COOKED.get() {
            libc::tcsetattr(0, libc::TCSANOW, cooked);
        }
        // Ends the process once this handler returns and the signal is
        // unblocked again.
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        // SAFETY: tcgetattr writes the termios it is given.
        let mut cooked: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(0, &raw mut cooked) } == 0 {
            let _ = COOKED.set(cooked);
        }
        let mut previous = Vec::new();
        for signal in ENDING_SIGNALS {
            // SAFETY: sigaction with a handler that only makes
            // async-signal-safe calls; the old action is kept to put back.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_ending_signal as extern "C" fn(libc::c_int) as usize;
                libc::sigemptyset(&raw mut action.sa_mask);
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, &raw const action, &raw mut old) == 0 {
                    previous.push((signal, old));
                }
            }
        }
        let guard = Self { previous };
        terminal::enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            DisableLineWrap,
            EnableBracketedPaste,
        )?;
        Ok(guard)
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
        for (signal, old) in &self.previous {
            // SAFETY: putting back the action sigaction gave us.
            unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()) };
        }
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
            copy: Incoming::default(),
        }
    }

    fn chunk(
        transfer_id: u64,
        mime: &str,
        total_size: u64,
        data: &[u8],
        last: bool,
    ) -> ClipboardWrite {
        ClipboardWrite {
            transfer_id,
            pane_id: "p".into(),
            mime: mime.into(),
            total_size,
            data: bytes::Bytes::copy_from_slice(data),
            last,
        }
    }

    #[test]
    fn copied_text_goes_to_the_terminal_as_osc_52() {
        let mut copy = Incoming::default();
        let text = "text/plain;charset=utf-8";
        assert_eq!(copy.take(chunk(1, text, 2, b"h", false)), None);
        assert_eq!(
            copy.take(chunk(1, text, 2, b"i", true)).as_deref(),
            Some(&b"\x1b]52;c;aGk=\x1b\\"[..])
        );
        // A copy cut short by a newer one is dropped, and so is a chunk of
        // it that comes late.
        assert_eq!(copy.take(chunk(2, text, 8, b"lost", false)), None);
        assert_eq!(copy.take(chunk(3, text, 5, b"the", false)), None);
        assert_eq!(copy.take(chunk(2, text, 8, b"late", false)), None);
        assert_eq!(
            copy.take(chunk(3, text, 5, b"re", true)).as_deref(),
            Some(&b"\x1b]52;c;dGhlcmU=\x1b\\"[..])
        );
        // Not text, or more than announced: nothing.
        assert_eq!(copy.take(chunk(4, "image/png", 3, b"png", true)), None);
        assert_eq!(copy.take(chunk(5, text, 1, b"too much", true)), None);
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
