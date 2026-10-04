use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Context, Result};
use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellWide, Screen as ActiveScreen};
use libghostty_vt::selection::Selection;
use libghostty_vt::style::{Style as VtStyle, StyleColor, Underline as VtUnderline};
use libghostty_vt::terminal::{Mode, ModeKind, Point, PointCoordinate, ScrollViewport};
use libghostty_vt::{RenderState, Terminal, TerminalOptions, key, mouse, paste};
use tilr_proto::{
    Color, Cursor, CursorShape, Frame, KeyAction, KeyInput, MouseAction, MouseButton, MouseInput,
    NamedKey, PaneStatus, Row, Run, Style, Underline, attrs, color, mods,
};

use crate::Screen;

const MAX_SCROLLBACK: usize = 10_000;
/// Nominal cell size in pixels. Clients draw in cells, but pixel-based
/// protocols (size reports, SGR-pixel mouse) still need a consistent answer.
const CELL_WIDTH_PX: u32 = 10;
const CELL_HEIGHT_PX: u32 = 20;

pub struct GhosttyScreen {
    // Field order matters: iterators and render state are dropped before the
    // terminal they were filled from.
    cells: CellIterator<'static>,
    rows: RowIterator<'static>,
    render: RenderState<'static>,
    key_encoder: key::Encoder<'static>,
    key_event: key::Event<'static>,
    mouse_encoder: mouse::Encoder<'static>,
    mouse_event: mouse::Event<'static>,
    term: Terminal<'static, 'static>,

    replies: Rc<RefCell<Vec<u8>>>,
    title_changed: Rc<Cell<bool>>,
    status_changed: Rc<Cell<bool>>,
    last_cursor: Option<Cursor>,
    last_modes: Option<(bool, bool)>,
    size: (u16, u16),
}

impl GhosttyScreen {
    pub fn new(cols: u16, rows: u16) -> Result<Self> {
        let mut term = new_terminal(cols, rows, MAX_SCROLLBACK)?;

        let replies = Rc::new(RefCell::new(Vec::new()));
        let title_changed = Rc::new(Cell::new(false));
        let status_changed = Rc::new(Cell::new(false));
        {
            let replies = Rc::clone(&replies);
            term.on_pty_write(move |_term: &Terminal<'_, '_>, data: &[u8]| {
                replies.borrow_mut().extend_from_slice(data);
            })?;
            let (title, status) = (Rc::clone(&title_changed), Rc::clone(&status_changed));
            term.on_title_changed(move |_term: &Terminal<'_, '_>| {
                title.set(true);
                status.set(true);
            })?;
            let status = Rc::clone(&status_changed);
            term.on_pwd_changed(move |_term: &Terminal<'_, '_>| status.set(true))?;
        }

        Ok(Self {
            cells: CellIterator::new()?,
            rows: RowIterator::new()?,
            render: RenderState::new()?,
            key_encoder: key::Encoder::new()?,
            key_event: key::Event::new()?,
            mouse_encoder: mouse::Encoder::new()?,
            mouse_event: mouse::Event::new()?,
            term,
            replies,
            title_changed,
            status_changed,
            last_cursor: None,
            last_modes: None,
            size: (cols, rows),
        })
    }

    /// Rebuild a screen from what [`Screen::snapshot`] returned for a screen
    /// of the same size.
    pub fn restore(cols: u16, rows: u16, snapshot: &[u8]) -> Result<Self> {
        let mut screen = Self::new(cols, rows)?;
        screen.feed(snapshot);
        // Nobody is waiting for answers to anything in a snapshot.
        screen.take_replies();
        Ok(screen)
    }
}

impl Screen for GhosttyScreen {
    fn feed(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
    }

    fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut *self.replies.borrow_mut())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.term
            .resize(cols, rows, CELL_WIDTH_PX, CELL_HEIGHT_PX)
            .context("resize terminal")?;
        self.size = (cols, rows);
        Ok(())
    }

    fn size(&self) -> (u16, u16) {
        self.size
    }

    fn scroll(&mut self, delta: i32) {
        self.term
            .scroll_viewport(ScrollViewport::Delta(delta as isize));
    }

    fn scroll_to_bottom(&mut self) {
        self.term.scroll_viewport(ScrollViewport::Bottom);
    }

    fn scrolled_back(&self) -> u32 {
        self.term.scrollbar().map_or(0, |bar| {
            bar.total
                .saturating_sub(bar.offset + bar.len)
                .min(u64::from(u32::MAX)) as u32
        })
    }

    fn frame(&mut self, full: bool) -> Result<Option<Frame>> {
        let modes = (
            self.term.is_mouse_tracking()?,
            self.term.active_screen()? == ActiveScreen::Alternate,
        );
        let title = (full || self.title_changed.replace(false))
            .then(|| self.term.title().map(str::to_owned))
            .transpose()?;

        let snapshot = self.render.update(&self.term)?;
        let all_rows = full || snapshot.dirty()? == Dirty::Full;

        let position = snapshot.cursor_viewport()?;
        let cursor = Cursor {
            visible: snapshot.cursor_visible()? && position.is_some(),
            x: position.map_or(0, |p| u32::from(p.x)),
            y: position.map_or(0, |p| u32::from(p.y)),
            shape: match snapshot.cursor_visual_style()? {
                CursorVisualStyle::Bar => CursorShape::Bar,
                CursorVisualStyle::Underline => CursorShape::Underline,
                CursorVisualStyle::BlockHollow => CursorShape::BlockHollow,
                _ => CursorShape::Block,
            } as i32,
            blinking: snapshot.cursor_blinking()?,
        };

        let mut styles = StyleTable::default();
        let mut changed = Vec::new();
        let mut row_iter = self.rows.update(&snapshot)?;
        let mut y = 0u32;
        while let Some(row) = row_iter.next() {
            if all_rows || row.dirty()? {
                let mut runs = RunBuilder::default();
                let mut cell_iter = self.cells.update(row)?;
                while let Some(cell) = cell_iter.next() {
                    let raw = cell.raw_cell()?;
                    let wide = match raw.wide()? {
                        CellWide::SpacerTail => continue,
                        CellWide::Wide => true,
                        _ => false,
                    };
                    let mut style = if raw.has_styling()? {
                        style_to_proto(cell.style()?)
                    } else {
                        Style::default()
                    };
                    // Cells erased with a background color carry it in the
                    // cell itself, not in a style.
                    match raw.content_tag()? {
                        CellContentTag::BgColorPalette => {
                            style.bg = color_to_proto(StyleColor::Palette(raw.bg_color_palette()?));
                        }
                        CellContentTag::BgColorRgb => {
                            style.bg = color_to_proto(StyleColor::Rgb(raw.bg_color_rgb()?));
                        }
                        _ => {}
                    }
                    let style = styles.index(style);
                    match cell.graphemes_len()? {
                        0 => runs.push_char(' ', style, wide),
                        1 => {
                            let mut buf = ['\0'; 1];
                            cell.graphemes_buf(&mut buf)?;
                            runs.push_char(buf[0], style, wide);
                        }
                        _ => {
                            runs.push_cluster(cell.graphemes()?.into_iter().collect(), style, wide)
                        }
                    }
                }
                changed.push(Row {
                    y,
                    runs: runs.finish(),
                });
                row.set_dirty(false)?;
            }
            y += 1;
        }
        snapshot.set_dirty(Dirty::Clean)?;

        let unchanged = changed.is_empty()
            && title.is_none()
            && self.last_cursor.as_ref() == Some(&cursor)
            && self.last_modes == Some(modes);
        if unchanged && !full {
            return Ok(None);
        }
        self.last_cursor = Some(cursor);
        self.last_modes = Some(modes);

        Ok(Some(Frame {
            pane_id: 0,
            seq: 0,
            full,
            cols: u32::from(self.size.0),
            rows: u32::from(self.size.1),
            styles: styles.list,
            changed,
            cursor: Some(cursor),
            title,
            mouse_tracking: modes.0,
            alt_screen: modes.1,
        }))
    }

    fn take_status(&mut self) -> Option<PaneStatus> {
        if !self.status_changed.replace(false) {
            return None;
        }
        Some(PaneStatus {
            title: self.term.title().unwrap_or_default().to_owned(),
            cwd: path_from_pwd(self.term.pwd().unwrap_or_default()),
        })
    }

    fn encode_key(&mut self, input: &KeyInput, out: &mut Vec<u8>) -> Result<()> {
        let (key, unshifted) = map_key(input);
        let mods = map_mods(input.mods);
        let action = match input.action() {
            KeyAction::Press => key::Action::Press,
            KeyAction::Release => key::Action::Release,
            KeyAction::Repeat => key::Action::Repeat,
        };
        // The encoder derives control sequences from key + mods; text must be
        // printable only.
        let text = (!input.text.is_empty() && !input.text.chars().any(char::is_control))
            .then_some(input.text.as_str());
        // Shift that already produced the text ("A" for Shift+a) is consumed.
        let consumed = if text.is_some() {
            mods & key::Mods::SHIFT
        } else {
            key::Mods::empty()
        };

        self.key_event
            .set_action(action)
            .set_key(key)
            .set_mods(mods)
            .set_consumed_mods(consumed)
            .set_unshifted_codepoint(unshifted.unwrap_or('\0'))
            .set_utf8(text);
        self.key_encoder
            .set_options_from_terminal(&self.term)
            // Clients report Alt as a modifier, never as macOS Option text.
            .set_macos_option_as_alt(key::OptionAsAlt::True);
        let (encoder, event) = (&mut self.key_encoder, &self.key_event);
        encode_into(out, |buf| encoder.encode(event, buf))
    }

    fn encode_mouse(&mut self, input: &MouseInput, out: &mut Vec<u8>) -> Result<()> {
        let (cols, rows) = self.size;
        let button = match input.button() {
            MouseButton::None => None,
            MouseButton::Left => Some(mouse::Button::Left),
            MouseButton::Right => Some(mouse::Button::Right),
            MouseButton::Middle => Some(mouse::Button::Middle),
            MouseButton::WheelUp => Some(mouse::Button::Four),
            MouseButton::WheelDown => Some(mouse::Button::Five),
        };
        self.mouse_event
            .set_action(match input.action() {
                MouseAction::Press => mouse::Action::Press,
                MouseAction::Release => mouse::Action::Release,
                MouseAction::Motion => mouse::Action::Motion,
            })
            .set_button(button)
            .set_mods(map_mods(input.mods))
            .set_position(mouse::Position {
                // Aim at the middle of the cell.
                x: (input.x * CELL_WIDTH_PX + CELL_WIDTH_PX / 2) as f32,
                y: (input.y * CELL_HEIGHT_PX + CELL_HEIGHT_PX / 2) as f32,
            });
        self.mouse_encoder
            .set_options_from_terminal(&self.term)
            .set_size(mouse::EncoderSize {
                screen_width: u32::from(cols) * CELL_WIDTH_PX,
                screen_height: u32::from(rows) * CELL_HEIGHT_PX,
                cell_width: CELL_WIDTH_PX,
                cell_height: CELL_HEIGHT_PX,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            });
        let (encoder, event) = (&mut self.mouse_encoder, &self.mouse_event);
        encode_into(out, |buf| encoder.encode(event, buf))
    }

    fn encode_paste(&mut self, text: &str, out: &mut Vec<u8>) -> Result<()> {
        let bracketed = self.term.mode(Mode::BRACKETED_PASTE)?;
        let mut data = text.as_bytes().to_vec();
        // Bracketed paste adds a 6-byte prefix and suffix.
        let mut buf = vec![0u8; data.len() + 16];
        let written = paste::encode(&mut data, bracketed, &mut buf)?;
        out.extend_from_slice(&buf[..written]);
        Ok(())
    }

    fn snapshot(mut self) -> Result<Vec<u8>> {
        let (cols, rows) = self.size;
        let last = PointCoordinate {
            x: cols - 1,
            y: u32::from(rows) - 1,
        };

        // Everything about the active screen is read before the alternate
        // screen is left below.
        let defaults = new_terminal(cols, rows, 0)?;
        let mut modes = Vec::new();
        for &mode in SNAPSHOT_MODES {
            let on = self.term.mode(mode)?;
            if on != defaults.mode(mode)? {
                let private = if mode.kind() == ModeKind::Dec {
                    "?"
                } else {
                    ""
                };
                let set = if on { 'h' } else { 'l' };
                modes.extend(format!("\x1b[{private}{}{set}", mode.value()).bytes());
            }
        }
        let cursor = cursor_position(&self.term)?;
        let cursor_shape = {
            let snapshot = self.render.update(&self.term)?;
            let steady = u8::from(!snapshot.cursor_blinking()?);
            match snapshot.cursor_visual_style()? {
                CursorVisualStyle::Underline => 3 + steady,
                CursorVisualStyle::Bar => 5 + steady,
                _ => 1 + steady,
            }
        };
        let title: String = self
            .term
            .title()?
            .chars()
            .filter(|ch| !ch.is_control())
            .collect();
        // The formatter only emits terminal state around some content, so it
        // is given the top-left cell, which is then printed onto itself.
        let state = format_range(&self.term, Point::Active, (0, 0), (0, 0), true)?;

        let alternate = if self.term.active_screen()? == ActiveScreen::Alternate {
            let content = format_range(&self.term, Point::Active, (0, 0), (last.x, last.y), false)?;
            let mode = [Mode::ALT_SCREEN_SAVE, Mode::ALT_SCREEN]
                .into_iter()
                .find(|&mode| self.term.mode(mode).unwrap_or(false))
                .unwrap_or(Mode::ALT_SCREEN_LEGACY)
                .value();
            // Back to the primary screen the way the application would go,
            // which also puts its cursor back.
            self.term.vt_write(format!("\x1b[?{mode}l").as_bytes());
            Some((mode, content))
        } else {
            None
        };

        let mut out = Vec::new();
        let history = self.term.scrollback_rows()?;
        if history > 0 {
            let end = (last.x, history as u32 - 1);
            out.extend(format_range(
                &self.term,
                Point::History,
                (0, 0),
                end,
                false,
            )?);
            // Push the history off the screen before the screen is drawn.
            out.extend(b"\x1b[0m");
            out.extend(b"\r\n".repeat(usize::from(rows)));
        }
        out.extend(b"\x1b[H");
        out.extend(format_range(
            &self.term,
            Point::Active,
            (0, 0),
            (last.x, last.y),
            false,
        )?);
        out.extend(b"\x1b[0m");
        out.extend(cursor_position(&self.term)?);
        if let Some((mode, content)) = alternate {
            out.extend(format!("\x1b[?{mode}h\x1b[H").bytes());
            out.extend(content);
            out.extend(b"\x1b[0m");
        }
        out.extend(b"\x1b[H");
        out.extend(state);
        out.extend(modes);
        out.extend(format!("\x1b]2;{title}\x1b\\\x1b[{cursor_shape} q").bytes());
        out.extend(cursor);
        Ok(out)
    }
}

/// Modes carried over by a snapshot. Left out: the alternate screen modes
/// (the snapshot switches screens itself), 132-column mode (setting it clears
/// the screen) and synchronized output (transient).
const SNAPSHOT_MODES: &[Mode] = &[
    Mode::KAM,
    Mode::INSERT,
    Mode::SRM,
    Mode::LINEFEED,
    Mode::DECCKM,
    Mode::REVERSE_COLORS,
    Mode::ORIGIN,
    Mode::WRAPAROUND,
    Mode::AUTOREPEAT,
    Mode::X10_MOUSE,
    Mode::CURSOR_BLINKING,
    Mode::CURSOR_VISIBLE,
    Mode::ENABLE_MODE3,
    Mode::REVERSE_WRAP,
    Mode::KEYPAD_KEYS,
    Mode::LEFT_RIGHT_MARGIN,
    Mode::NORMAL_MOUSE,
    Mode::BUTTON_MOUSE,
    Mode::ANY_MOUSE,
    Mode::FOCUS_EVENT,
    Mode::UTF8_MOUSE,
    Mode::SGR_MOUSE,
    Mode::ALT_SCROLL,
    Mode::URXVT_MOUSE,
    Mode::SGR_PIXELS_MOUSE,
    Mode::NUMLOCK_KEYPAD,
    Mode::ALT_ESC_PREFIX,
    Mode::ALT_SENDS_ESC,
    Mode::REVERSE_WRAP_EXT,
    Mode::BRACKETED_PASTE,
    Mode::GRAPHEME_CLUSTER,
    Mode::COLOR_SCHEME_REPORT,
    Mode::IN_BAND_RESIZE,
];

fn new_terminal(cols: u16, rows: u16, max_scrollback: usize) -> Result<Terminal<'static, 'static>> {
    let mut term = Terminal::new(TerminalOptions {
        cols,
        rows,
        max_scrollback,
    })
    .context("create terminal")?;
    term.resize(cols, rows, CELL_WIDTH_PX, CELL_HEIGHT_PX)?;
    // Keep multi-codepoint graphemes (ZWJ emoji, flags) in one cell. A full
    // reset (RIS) turns this back off; fixing that needs a patched
    // libghostty-vt.
    term.set_mode(Mode::GRAPHEME_CLUSTER, true)?;
    Ok(term)
}

/// The sequence that moves the cursor to where it is in `term`.
fn cursor_position(term: &Terminal<'_, '_>) -> Result<Vec<u8>> {
    let (x, y) = (term.cursor_x()?, term.cursor_y()?);
    Ok(format!("\x1b[{};{}H", y + 1, x + 1).into_bytes())
}

/// Format the cells from `start` to `end` (inclusive, as `(x, y)` in the
/// space of `point`) as VT sequences. With `state`, the terminal state that
/// lives outside the cells comes along: scrolling region, tabstops, keyboard
/// protocol, charsets, working directory and the current style.
fn format_range(
    term: &Terminal<'_, '_>,
    point: fn(PointCoordinate) -> Point,
    start: (u16, u32),
    end: (u16, u32),
    state: bool,
) -> Result<Vec<u8>> {
    let start = term.grid_ref(point(PointCoordinate {
        x: start.0,
        y: start.1,
    }))?;
    let end = term.grid_ref(point(PointCoordinate { x: end.0, y: end.1 }))?;
    let selection = Selection::new(start, end, false);
    let options = FormatterOptions::new()
        .with_format(Format::Vt)
        .with_unwrap(true)
        .with_trim(false)
        .with_selection(&selection)
        .with_palette(state)
        .with_scrolling_region(state)
        .with_tabstops(state)
        .with_pwd(state)
        .with_keyboard(state)
        .with_style(state)
        .with_hyperlink(state)
        .with_protection(state)
        .with_kitty_keyboard(state)
        .with_charsets(state);
    let mut formatter = Formatter::new(term, options)?;
    Ok(formatter.format_alloc(None)?.to_vec())
}

/// Run a libghostty encoder and append its output to `out`.
///
/// The crate's own `encode_to_vec` is not used: it under-reserves when the
/// vector already holds data and then fails with `OutOfSpace`.
fn encode_into(
    out: &mut Vec<u8>,
    mut encode: impl FnMut(&mut [u8]) -> libghostty_vt::error::Result<usize>,
) -> Result<()> {
    let mut stack = [0u8; 128];
    match encode(&mut stack) {
        Ok(written) => out.extend_from_slice(&stack[..written]),
        Err(libghostty_vt::Error::OutOfSpace { required }) => {
            let mut heap = vec![0u8; required];
            let written = encode(&mut heap)?;
            out.extend_from_slice(&heap[..written]);
        }
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

/// Deduplicates styles within one frame. Index 0 is the default style and is
/// never stored.
#[derive(Default)]
struct StyleTable {
    list: Vec<Style>,
    index: HashMap<StyleKey, u32>,
}

type StyleKey = (u64, u64, u64, u32, i32);

impl StyleTable {
    fn index(&mut self, style: Style) -> u32 {
        if style == Style::default() {
            return 0;
        }
        let key = (
            color_key(style.fg.as_ref()),
            color_key(style.bg.as_ref()),
            color_key(style.underline_color.as_ref()),
            style.attrs,
            style.underline,
        );
        *self.index.entry(key).or_insert_with(|| {
            self.list.push(style);
            self.list.len() as u32
        })
    }
}

fn color_key(color: Option<&Color>) -> u64 {
    match color.and_then(|c| c.value) {
        None => 0,
        Some(color::Value::Palette(index)) => 1 << 32 | u64::from(index),
        Some(color::Value::Rgb(rgb)) => 2 << 32 | u64::from(rgb),
    }
}

#[derive(Default)]
struct RunBuilder {
    runs: Vec<Run>,
}

impl RunBuilder {
    fn push_char(&mut self, ch: char, style: u32, wide: bool) {
        match self.runs.last_mut() {
            Some(run) if run.style == style && run.wide == wide && !run.cluster => {
                run.text.push(ch);
            }
            _ => self.runs.push(Run {
                style,
                text: ch.to_string(),
                wide,
                cluster: false,
            }),
        }
    }

    fn push_cluster(&mut self, text: String, style: u32, wide: bool) {
        self.runs.push(Run {
            style,
            text,
            wide,
            cluster: true,
        });
    }

    /// Drop trailing blank default-style cells; the client clears the rest of
    /// the row anyway.
    fn finish(mut self) -> Vec<Run> {
        while let Some(run) = self.runs.last_mut() {
            if run.style != 0 || run.wide || run.cluster {
                break;
            }
            let trimmed = run.text.trim_end_matches(' ').len();
            run.text.truncate(trimmed);
            if !run.text.is_empty() {
                break;
            }
            self.runs.pop();
        }
        self.runs
    }
}

/// Shells report the working directory as a `file://host/path` URL (OSC 7).
fn path_from_pwd(pwd: &str) -> String {
    let Some(rest) = pwd.strip_prefix("file://") else {
        return pwd.to_owned();
    };
    let path = rest.find('/').map_or("", |start| &rest[start..]);
    let mut out = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hex = [bytes.next(), bytes.next()];
            if let [Some(hi), Some(lo)] = hex
                && let Ok(decoded) = u8::from_str_radix(&String::from_utf8_lossy(&[hi, lo]), 16)
            {
                out.push(decoded);
                continue;
            }
            out.push(b'%');
            out.extend(hex.into_iter().flatten());
        } else {
            out.push(byte);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn style_to_proto(style: VtStyle) -> Style {
    let mut bits = 0;
    for (on, bit) in [
        (style.bold, attrs::BOLD),
        (style.italic, attrs::ITALIC),
        (style.faint, attrs::FAINT),
        (style.blink, attrs::BLINK),
        (style.inverse, attrs::INVERSE),
        (style.invisible, attrs::INVISIBLE),
        (style.strikethrough, attrs::STRIKETHROUGH),
        (style.overline, attrs::OVERLINE),
    ] {
        if on {
            bits |= bit;
        }
    }
    Style {
        fg: color_to_proto(style.fg_color),
        bg: color_to_proto(style.bg_color),
        underline_color: color_to_proto(style.underline_color),
        attrs: bits,
        underline: match style.underline {
            VtUnderline::Single => Underline::Single,
            VtUnderline::Double => Underline::Double,
            VtUnderline::Curly => Underline::Curly,
            VtUnderline::Dotted => Underline::Dotted,
            VtUnderline::Dashed => Underline::Dashed,
            _ => Underline::None,
        } as i32,
    }
}

fn color_to_proto(color: StyleColor) -> Option<Color> {
    let value = match color {
        StyleColor::None => return None,
        StyleColor::Palette(index) => color::Value::Palette(u32::from(index.0)),
        StyleColor::Rgb(rgb) => {
            color::Value::Rgb(u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b))
        }
    };
    Some(Color { value: Some(value) })
}

fn map_mods(bits: u32) -> key::Mods {
    let mut out = key::Mods::empty();
    for (bit, flag) in [
        (mods::SHIFT, key::Mods::SHIFT),
        (mods::ALT, key::Mods::ALT),
        (mods::CTRL, key::Mods::CTRL),
        (mods::SUPER, key::Mods::SUPER),
    ] {
        if bits & bit != 0 {
            out |= flag;
        }
    }
    out
}

/// Map a protocol key to a physical key plus its unshifted codepoint.
fn map_key(input: &KeyInput) -> (key::Key, Option<char>) {
    use key::Key;

    let named = match input.named() {
        NamedKey::Unspecified => None,
        NamedKey::Enter => Some(Key::Enter),
        NamedKey::Tab => Some(Key::Tab),
        NamedKey::Backspace => Some(Key::Backspace),
        NamedKey::Escape => Some(Key::Escape),
        NamedKey::Space => return (Key::Space, Some(' ')),
        NamedKey::Up => Some(Key::ArrowUp),
        NamedKey::Down => Some(Key::ArrowDown),
        NamedKey::Left => Some(Key::ArrowLeft),
        NamedKey::Right => Some(Key::ArrowRight),
        NamedKey::Home => Some(Key::Home),
        NamedKey::End => Some(Key::End),
        NamedKey::PageUp => Some(Key::PageUp),
        NamedKey::PageDown => Some(Key::PageDown),
        NamedKey::Insert => Some(Key::Insert),
        NamedKey::Delete => Some(Key::Delete),
        NamedKey::F1 => Some(Key::F1),
        NamedKey::F2 => Some(Key::F2),
        NamedKey::F3 => Some(Key::F3),
        NamedKey::F4 => Some(Key::F4),
        NamedKey::F5 => Some(Key::F5),
        NamedKey::F6 => Some(Key::F6),
        NamedKey::F7 => Some(Key::F7),
        NamedKey::F8 => Some(Key::F8),
        NamedKey::F9 => Some(Key::F9),
        NamedKey::F10 => Some(Key::F10),
        NamedKey::F11 => Some(Key::F11),
        NamedKey::F12 => Some(Key::F12),
    };
    if let Some(key) = named {
        return (key, None);
    }

    let Some(ch) = char::from_u32(input.codepoint).filter(|&c| c != '\0') else {
        return (Key::Unidentified, None);
    };
    let ch = ch.to_ascii_lowercase();
    let key = match ch {
        'a'..='z' => Key::try_from(Key::A as u32 + (ch as u32 - 'a' as u32)).ok(),
        '0'..='9' => Key::try_from(Key::Digit0 as u32 + (ch as u32 - '0' as u32)).ok(),
        ' ' => Some(Key::Space),
        '`' => Some(Key::Backquote),
        '\\' => Some(Key::Backslash),
        '[' => Some(Key::BracketLeft),
        ']' => Some(Key::BracketRight),
        ',' => Some(Key::Comma),
        '=' => Some(Key::Equal),
        '-' => Some(Key::Minus),
        '.' => Some(Key::Period),
        '\'' => Some(Key::Quote),
        ';' => Some(Key::Semicolon),
        '/' => Some(Key::Slash),
        _ => None,
    };
    (key.unwrap_or(Key::Unidentified), Some(ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> GhosttyScreen {
        GhosttyScreen::new(20, 4).unwrap()
    }

    fn row_text(frame: &Frame, y: u32) -> String {
        frame
            .changed
            .iter()
            .find(|row| row.y == y)
            .map(|row| row.runs.iter().map(|run| run.text.as_str()).collect())
            .unwrap_or_default()
    }

    fn key(named: NamedKey, codepoint: char, mods: u32, text: &str) -> KeyInput {
        KeyInput {
            pane_id: 0,
            action: KeyAction::Press as i32,
            mods,
            named: named as i32,
            codepoint: codepoint as u32,
            text: text.to_owned(),
        }
    }

    fn encoded(screen: &mut GhosttyScreen, input: KeyInput) -> Vec<u8> {
        let mut out = Vec::new();
        screen.encode_key(&input, &mut out).unwrap();
        out
    }

    #[test]
    fn full_frame_has_every_row_and_trims_blanks() {
        let mut screen = screen();
        screen.feed(b"hello");
        let frame = screen.frame(true).unwrap().unwrap();
        assert!(frame.full);
        assert_eq!(frame.changed.len(), 4);
        assert_eq!(row_text(&frame, 0), "hello");
        assert!(frame.changed[1].runs.is_empty());
        let cursor = frame.cursor.unwrap();
        assert_eq!((cursor.x, cursor.y, cursor.visible), (5, 0, true));
    }

    #[test]
    fn diff_frame_has_only_changed_rows() {
        let mut screen = screen();
        screen.feed(b"one\r\ntwo");
        screen.frame(true).unwrap();
        assert!(screen.frame(false).unwrap().is_none());

        screen.feed(b"!");
        let frame = screen.frame(false).unwrap().unwrap();
        assert!(!frame.full);
        assert_eq!(frame.changed.len(), 1);
        assert_eq!(frame.changed[0].y, 1);
        assert_eq!(row_text(&frame, 1), "two!");
    }

    #[test]
    fn cursor_move_alone_produces_frame() {
        let mut screen = screen();
        screen.feed(b"abc");
        screen.frame(true).unwrap();
        screen.feed(b"\x1b[D");
        let frame = screen.frame(false).unwrap().unwrap();
        assert_eq!(frame.cursor.unwrap().x, 2);
    }

    #[test]
    fn styles_are_split_into_runs() {
        let mut screen = screen();
        screen.feed(b"a\x1b[1;31mb\x1b[0mc");
        let frame = screen.frame(true).unwrap().unwrap();
        let runs = &frame.changed[0].runs;
        assert_eq!(runs.len(), 3);
        assert_eq!((runs[0].style, runs[2].style), (0, 0));
        let style = &frame.styles[runs[1].style as usize - 1];
        assert_eq!(style.attrs, attrs::BOLD);
        assert_eq!(style.fg.unwrap().value, Some(color::Value::Palette(1)));
    }

    #[test]
    fn wide_and_cluster_cells() {
        let mut screen = screen();
        screen.feed("a\u{4e16}b".as_bytes());
        let frame = screen.frame(true).unwrap().unwrap();
        let runs = &frame.changed[0].runs;
        assert_eq!(runs.len(), 3);
        assert!(runs[1].wide);
        assert_eq!(runs[1].text, "\u{4e16}");
        assert_eq!(runs[2].text, "b");

        screen.feed("\r\ne\u{301}".as_bytes());
        let frame = screen.frame(false).unwrap().unwrap();
        let row = frame.changed.iter().find(|row| row.y == 1).unwrap();
        let run = &row.runs[0];
        assert!(run.cluster);
        assert_eq!(run.text, "e\u{301}");
    }

    #[test]
    fn title_reported_on_change() {
        let mut screen = screen();
        screen.frame(true).unwrap();
        screen.feed(b"\x1b]2;hi\x07");
        let frame = screen.frame(false).unwrap().unwrap();
        assert_eq!(frame.title.as_deref(), Some("hi"));
    }

    #[test]
    fn status_reports_title_and_cwd() {
        let mut screen = screen();
        assert!(screen.take_status().is_none());
        screen.feed(b"\x1b]2;vim\x07\x1b]7;file://host/tmp/a%20b\x07");
        let status = screen.take_status().unwrap();
        assert_eq!(
            (status.title.as_str(), status.cwd.as_str()),
            ("vim", "/tmp/a b")
        );
        assert!(screen.take_status().is_none());
    }

    #[test]
    fn replies_to_queries() {
        let mut screen = screen();
        screen.feed(b"\x1b[6n");
        assert_eq!(screen.take_replies(), b"\x1b[1;1R");
        assert!(screen.take_replies().is_empty());
    }

    #[test]
    fn scrollback_viewport() {
        let mut screen = screen();
        for i in 0..10 {
            screen.feed(format!("line{i}\r\n").as_bytes());
        }
        screen.frame(true).unwrap();
        screen.scroll(-3);
        let frame = screen.frame(false).unwrap().unwrap();
        assert_eq!(row_text(&frame, 0), "line4");
        assert!(!frame.cursor.unwrap().visible);
        assert_eq!(screen.scrolled_back(), 3);
        screen.scroll_to_bottom();
        let frame = screen.frame(false).unwrap().unwrap();
        assert_eq!(row_text(&frame, 0), "line7");
        assert_eq!(screen.scrolled_back(), 0);
        // Scrolling stops at the first line of history.
        screen.scroll(-1000);
        assert_eq!(screen.scrolled_back(), 7);
    }

    #[test]
    fn encodes_keys() {
        let mut screen = screen();
        let none = NamedKey::Unspecified;
        assert_eq!(encoded(&mut screen, key(none, 'a', 0, "a")), b"a");
        assert_eq!(encoded(&mut screen, key(none, 'a', mods::SHIFT, "A")), b"A");
        assert_eq!(
            encoded(&mut screen, key(none, 'c', mods::CTRL, "")),
            b"\x03"
        );
        assert_eq!(
            encoded(&mut screen, key(none, 'a', mods::ALT, "a")),
            b"\x1ba"
        );
        assert_eq!(
            encoded(&mut screen, key(NamedKey::Enter, '\0', 0, "")),
            b"\r"
        );
        assert_eq!(
            encoded(&mut screen, key(NamedKey::Up, '\0', 0, "")),
            b"\x1b[A"
        );
        assert_eq!(
            encoded(&mut screen, key(none, '\u{44f}', 0, "\u{44f}")),
            "\u{44f}".as_bytes()
        );
    }

    /// Input is appended to a buffer that may still hold earlier, unsent
    /// input and have only a few spare bytes.
    #[test]
    fn encoding_appends_to_a_nearly_full_buffer() {
        let mut screen = screen();
        screen.feed(b"\x1b[?1000h\x1b[?1006h");
        let click = MouseInput {
            pane_id: 0,
            action: MouseAction::Press as i32,
            button: MouseButton::Left as i32,
            mods: 0,
            x: 2,
            y: 1,
        };
        for spare in 0..16 {
            let mut out = Vec::with_capacity(5 + spare);
            out.extend_from_slice(b"12345");
            screen.encode_mouse(&click, &mut out).unwrap();
            screen
                .encode_key(
                    &key(NamedKey::F5, '\0', mods::CTRL | mods::SHIFT, ""),
                    &mut out,
                )
                .unwrap();
            assert!(
                out.starts_with(b"12345\x1b[<0;3;2M\x1b["),
                "spare={spare}: {out:?}"
            );
        }
    }

    #[test]
    fn arrow_keys_follow_application_mode() {
        let mut screen = screen();
        screen.feed(b"\x1b[?1h");
        assert_eq!(
            encoded(&mut screen, key(NamedKey::Up, '\0', 0, "")),
            b"\x1bOA"
        );
    }

    #[test]
    fn paste_respects_bracketed_mode() {
        let mut screen = screen();
        let mut out = Vec::new();
        screen.encode_paste("hi", &mut out).unwrap();
        assert_eq!(out, b"hi");

        screen.feed(b"\x1b[?2004h");
        out.clear();
        screen.encode_paste("hi", &mut out).unwrap();
        assert_eq!(out, b"\x1b[200~hi\x1b[201~");
    }

    #[test]
    fn mouse_encoded_only_when_tracking() {
        let mut screen = screen();
        let click = MouseInput {
            pane_id: 0,
            action: MouseAction::Press as i32,
            button: MouseButton::Left as i32,
            mods: 0,
            x: 2,
            y: 1,
        };
        let mut out = Vec::new();
        screen.encode_mouse(&click, &mut out).unwrap();
        assert!(out.is_empty());

        screen.feed(b"\x1b[?1000h\x1b[?1006h");
        assert!(screen.frame(true).unwrap().unwrap().mouse_tracking);
        screen.encode_mouse(&click, &mut out).unwrap();
        assert_eq!(out, b"\x1b[<0;3;2M");
    }

    /// What a client can see of a screen: the rows at the bottom, the rows
    /// at the top of the history, the cursor and the modes.
    fn view(screen: &mut GhosttyScreen) -> (Vec<String>, Vec<String>, Option<Cursor>, bool, bool) {
        let rows = |frame: &Frame| (0..frame.rows).map(|y| row_text(frame, y)).collect();
        screen.scroll_to_bottom();
        let bottom = screen.frame(true).unwrap().unwrap();
        screen.scroll(-100);
        let top = screen.frame(true).unwrap().unwrap();
        screen.scroll_to_bottom();
        (
            rows(&bottom),
            rows(&top),
            bottom.cursor,
            bottom.mouse_tracking,
            bottom.alt_screen,
        )
    }

    /// Snapshot and restore a screen fed with `bytes`, checking that nothing
    /// visible changed. Returns the restored screen.
    fn round_trip(bytes: &[u8]) -> GhosttyScreen {
        let mut original = screen();
        original.feed(bytes);
        let before = view(&mut original);
        let history = original.term.scrollback_rows().unwrap();

        let snapshot = original.snapshot().unwrap();
        let mut restored = GhosttyScreen::restore(20, 4, &snapshot).unwrap();
        assert_eq!(
            view(&mut restored),
            before,
            "snapshot: {:?}",
            String::from_utf8_lossy(&snapshot)
        );
        assert_eq!(restored.term.scrollback_rows().unwrap(), history);
        restored
    }

    #[test]
    fn snapshot_keeps_scrollback_and_styles() {
        let mut bytes = Vec::new();
        for i in 0..10 {
            bytes.extend(format!("line {i}\r\n").bytes());
        }
        bytes.extend(b"\x1b[31mred\x1b[0m and a long line that wraps around the edge");
        let mut restored = round_trip(&bytes);
        let frame = restored.frame(true).unwrap().unwrap();
        assert_eq!(frame.styles.len(), 1);
    }

    #[test]
    fn snapshot_keeps_a_cleared_screen_below_its_history() {
        let mut bytes = Vec::new();
        for i in 0..10 {
            bytes.extend(format!("line {i}\r\n").bytes());
        }
        bytes.extend(b"\x1b[2J\x1b[Hprompt");
        round_trip(&bytes);
    }

    #[test]
    fn snapshot_keeps_both_screens() {
        let mut restored =
            round_trip(b"one\r\ntwo\r\n$ app\r\n\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[2;3Halt");
        // The application leaves: the primary screen and its cursor are back.
        restored.feed(b"\x1b[?1000l\x1b[?25h\x1b[?1049l");
        let frame = restored.frame(true).unwrap().unwrap();
        assert!(!frame.alt_screen);
        assert_eq!(row_text(&frame, 2), "$ app");
        let cursor = frame.cursor.unwrap();
        assert_eq!((cursor.x, cursor.y, cursor.visible), (0, 3, true));
    }

    #[test]
    fn snapshot_keeps_modes_title_and_keyboard_protocol() {
        let mut restored = round_trip(b"\x1b]2;my title\x07\x1b[?2004h\x1b[?1h\x1b[>5u\x1b[4 qhi");
        assert_eq!(restored.term.title().unwrap(), "my title");
        assert!(restored.term.mode(Mode::BRACKETED_PASTE).unwrap());
        assert!(restored.term.mode(Mode::DECCKM).unwrap());
        assert_eq!(restored.term.kitty_keyboard_flags().unwrap().bits(), 5);
        let cursor = restored.frame(true).unwrap().unwrap().cursor.unwrap();
        assert_eq!(
            (cursor.shape, cursor.blinking),
            (CursorShape::Underline as i32, false)
        );
    }
}
