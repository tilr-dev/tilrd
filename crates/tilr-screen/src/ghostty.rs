use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

use anyhow::{Context, Result};
use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::kitty::graphics;
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellWide, Screen as ActiveScreen};
use libghostty_vt::selection::Selection;
use libghostty_vt::style::{Style as VtStyle, StyleColor, Underline as VtUnderline};
use libghostty_vt::terminal::{Mode, ModeKind, Point, PointCoordinate, ScrollViewport};
use libghostty_vt::{RenderState, Terminal, TerminalOptions, key, mouse, paste};
use tilr_proto::{
    Color, Cursor, CursorShape, Frame, Image, KeyAction, KeyInput, MouseAction, MouseButton,
    MouseInput, NamedKey, PaneStatus, Placement, Row, Run, Style, Underline, attrs, color, mods,
};

use crate::Screen;

const MAX_SCROLLBACK: usize = 10_000;
/// The cell in pixels until a client says what its own is: image placements
/// are measured in it, and pixel-based protocols (size reports, SGR-pixel
/// mouse) need a consistent answer.
pub const DEFAULT_CELL_PX: (u32, u32) = (10, 20);
/// Kitty's unicode placeholder: a cell showing part of a virtual placement.
const PLACEHOLDER: char = '\u{10EEEE}';
/// Bytes of decoded image data a screen may hold (Kitty's default).
const KITTY_STORAGE_LIMIT: u64 = 320 * 1024 * 1024;

pub struct GhosttyScreen {
    // Field order matters: iterators and render state are dropped before the
    // terminal they were filled from.
    cells: CellIterator<'static>,
    rows: RowIterator<'static>,
    placements: graphics::PlacementIterator<'static>,
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
    last_placements: Vec<Placement>,
    last_viewport_top: u32,
    /// Every image in view, encoded once per (id, generation); dropped when
    /// it leaves the view.
    images: HashMap<(u32, u64), Image>,
    /// The runs of unicode placeholder cells on each row of the screen,
    /// kept up to date for the rows that change.
    placeholders: Vec<Vec<PlaceholderRun>>,
    size: (u16, u16),
    cell: (u32, u32),
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
            placements: graphics::PlacementIterator::new()?,
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
            last_placements: Vec::new(),
            last_viewport_top: 0,
            images: HashMap::new(),
            placeholders: Vec::new(),
            size: (cols, rows),
            cell: DEFAULT_CELL_PX,
        })
    }

    /// Rebuild a screen from what [`Screen::snapshot`] returned for a screen
    /// of the same size.
    pub fn restore(cols: u16, rows: u16, cell: (u32, u32), snapshot: &[u8]) -> Result<Self> {
        let mut screen = Self::new(cols, rows)?;
        if cell != DEFAULT_CELL_PX {
            screen.resize(cols, rows, cell)?;
        }
        screen.feed(snapshot);
        // Nobody is waiting for answers to anything in a snapshot.
        screen.take_replies();
        Ok(screen)
    }

    /// The Kitty graphics placements in the viewport, and the images among
    /// them that were not encoded before (`self.images` is brought up to
    /// date: what left the view is dropped).
    fn placements_in_view(&mut self) -> Result<(Vec<Placement>, Vec<Image>)> {
        let graphics = self.term.kitty_graphics()?;
        let mut placements = Vec::new();
        let mut new_images = Vec::new();
        let mut in_view = std::collections::HashSet::new();
        let mut virtuals = Vec::new();
        let mut iteration = self.placements.update(&graphics)?;
        while let Some(placement) = iteration.next() {
            let image_id = placement.image_id()?;
            if placement.is_virtual()? {
                // Shown through placeholder cells, found below.
                virtuals.push(VirtualPlacement {
                    image_id,
                    placement_id: placement.placement_id()?,
                    columns: placement.columns()?,
                    rows: placement.rows()?,
                    z: placement.z()?,
                });
                continue;
            }
            let Some(image) = graphics.image(image_id) else {
                continue;
            };
            let info = placement.placement_render_info(&image, &self.term)?;
            if !info.viewport_visible {
                continue;
            }
            let Some(key) = encode_cached(&mut self.images, &image, &mut new_images)? else {
                continue;
            };
            in_view.insert(key);
            placements.push(Placement {
                image_id,
                image_generation: key.1,
                col: info.viewport_col,
                row: info.viewport_row,
                cols: info.grid_cols,
                rows: info.grid_rows,
                source_x: info.source_x,
                source_y: info.source_y,
                source_width: info.source_width,
                source_height: info.source_height,
                x_offset: placement.x_offset()?,
                y_offset: placement.y_offset()?,
                z: placement.z()?,
                pixel_width: info.pixel_width,
                pixel_height: info.pixel_height,
            });
        }
        for (y, runs) in self.placeholders.iter().enumerate() {
            for run in runs {
                let Some(virt) = virtuals.iter().find(|v| {
                    v.image_id == run.image_id
                        && (run.placement_id == 0 || v.placement_id == run.placement_id)
                }) else {
                    continue;
                };
                let Some(image) = graphics.image(run.image_id) else {
                    continue;
                };
                let (width, height) = (image.width()?, image.height()?);
                let grid = (
                    if virt.columns > 0 {
                        virt.columns
                    } else {
                        width.div_ceil(self.cell.0)
                    },
                    if virt.rows > 0 {
                        virt.rows
                    } else {
                        height.div_ceil(self.cell.1)
                    },
                );
                let Some(part) = placeholder_geometry(run, grid, (width, height), self.cell) else {
                    continue;
                };
                let Some(key) = encode_cached(&mut self.images, &image, &mut new_images)? else {
                    continue;
                };
                in_view.insert(key);
                placements.push(Placement {
                    image_id: run.image_id,
                    image_generation: key.1,
                    col: run.x as i32,
                    row: y as i32,
                    cols: run.width,
                    rows: 1,
                    source_x: part.source.0,
                    source_y: part.source.1,
                    source_width: part.source.2,
                    source_height: part.source.3,
                    x_offset: part.offset.0,
                    y_offset: part.offset.1,
                    z: virt.z,
                    pixel_width: part.size.0,
                    pixel_height: part.size.1,
                });
            }
        }
        self.images.retain(|key, _| in_view.contains(key));
        Ok((placements, new_images))
    }
}

impl Screen for GhosttyScreen {
    fn feed(&mut self, bytes: &[u8]) {
        self.term.vt_write(bytes);
    }

    fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut *self.replies.borrow_mut())
    }

    fn resize(&mut self, cols: u16, rows: u16, cell: (u32, u32)) -> Result<()> {
        let cell = (cell.0.max(1), cell.1.max(1));
        self.term
            .resize(cols, rows, cell.0, cell.1)
            .context("resize terminal")?;
        self.size = (cols, rows);
        self.cell = cell;
        self.placeholders.clear();
        Ok(())
    }

    fn size(&self) -> (u16, u16) {
        self.size
    }

    fn cell_px(&self) -> (u32, u32) {
        self.cell
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
        self.placeholders
            .resize_with(usize::from(self.size.1), Vec::new);
        let mut row_iter = self.rows.update(&snapshot)?;
        let mut y = 0u32;
        while let Some(row) = row_iter.next() {
            if all_rows || row.dirty()? {
                let mut runs = RunBuilder::default();
                let mut placeholders = PlaceholderRuns::default();
                let mut column = 0u32;
                let mut cell_iter = self.cells.update(row)?;
                while let Some(cell) = cell_iter.next() {
                    let raw = cell.raw_cell()?;
                    let wide = match raw.wide()? {
                        CellWide::SpacerTail => continue,
                        CellWide::Wide => true,
                        _ => false,
                    };
                    let x = column;
                    column += if wide { 2 } else { 1 };
                    let styled = raw.has_styling()?;
                    let mut style = if styled {
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
                    let len = cell.graphemes_len()?;
                    let mut buf = ['\0'; 1];
                    if len == 1 {
                        cell.graphemes_buf(&mut buf)?;
                    }
                    if len > 0 && (len > 1 || buf[0] == PLACEHOLDER) {
                        let cps = cell.graphemes()?;
                        if cps[0] == PLACEHOLDER {
                            // Part of an image: a blank cell for the client, the
                            // image's row and column from the diacritics.
                            let vt_style = if styled { Some(cell.style()?) } else { None };
                            placeholders.push(x, &cps, vt_style);
                            runs.push_char(' ', style, wide);
                        } else {
                            runs.push_cluster(cps.into_iter().collect(), style, wide);
                        }
                        continue;
                    }
                    placeholders.end();
                    runs.push_char(if len == 0 { ' ' } else { buf[0] }, style, wide);
                }
                if let Some(slot) = self.placeholders.get_mut(y as usize) {
                    *slot = placeholders.finish();
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

        let (placements, new_images) = self.placements_in_view()?;
        let viewport_top = self
            .term
            .scrollbar()
            .map_or(0, |bar| bar.offset.min(u64::from(u32::MAX)) as u32);
        let placements_changed = full || placements != self.last_placements;
        let unchanged = changed.is_empty()
            && title.is_none()
            && self.last_cursor.as_ref() == Some(&cursor)
            && self.last_modes == Some(modes)
            && !placements_changed
            && viewport_top == self.last_viewport_top;
        if unchanged && !full {
            return Ok(None);
        }
        self.last_cursor = Some(cursor);
        self.last_modes = Some(modes);
        self.last_viewport_top = viewport_top;
        // A full frame goes to a subscriber that may know none of the images.
        let images = if full {
            self.images.values().cloned().collect()
        } else {
            new_images
        };
        if placements_changed {
            self.last_placements.clone_from(&placements);
        }

        Ok(Some(Frame {
            pane_id: String::new(),
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
            placements_changed,
            placements: if placements_changed {
                placements
            } else {
                Vec::new()
            },
            images,
            cell_width_px: self.cell.0,
            cell_height_px: self.cell.1,
            viewport_top,
            // Filled in by the pane process, which knows what led to the frame.
            last_input_seq: 0,
            timing: None,
        }))
    }

    fn unchanged_frame(&self) -> Option<Frame> {
        let cursor = self.last_cursor?;
        let (mouse_tracking, alt_screen) = self.last_modes?;
        Some(Frame {
            cols: u32::from(self.size.0),
            rows: u32::from(self.size.1),
            cursor: Some(cursor),
            mouse_tracking,
            alt_screen,
            cell_width_px: self.cell.0,
            cell_height_px: self.cell.1,
            viewport_top: self.last_viewport_top,
            ..Frame::default()
        })
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
        let (mut key, mut unshifted) = map_key(input);
        let mods = map_mods(input.mods);
        let action = match input.action() {
            KeyAction::Press => key::Action::Press,
            KeyAction::Release => key::Action::Release,
            KeyAction::Repeat => key::Action::Repeat,
        };
        // The encoder derives control sequences from key + mods; text must be
        // printable only.
        let mut text = (!input.text.is_empty() && !input.text.chars().any(char::is_control))
            .then(|| input.text.clone());
        if let Some((physical, us)) = physical_key(&input.code)
            && input.named() == NamedKey::Unspecified
        {
            let kitty = self
                .term
                .kitty_keyboard_flags()
                .is_ok_and(|flags| flags.bits() != 0);
            let character = unshifted.or_else(|| text.as_deref()?.chars().next());
            let ascii = character.is_none_or(|ch| ch.is_ascii());
            if kitty {
                // The key as the layout has it, and where it is: the kitty
                // protocol reports the physical key as the base layout key.
                key = physical;
                unshifted =
                    unshifted.or(character.map(|ch| ch.to_lowercase().next().unwrap_or(ch)));
            } else if !ascii && input.mods & (mods::CTRL | mods::ALT) != 0 {
                // Ctrl or Alt on a layout that is not Latin: what terminals
                // send is the key's on a US layout (Ctrl+С is ^C).
                key = physical;
                unshifted = Some(us);
                text = (input.mods & mods::CTRL == 0).then(|| {
                    if input.mods & mods::SHIFT != 0 {
                        us.to_ascii_uppercase().to_string()
                    } else {
                        us.to_string()
                    }
                });
            }
        }
        let text = text.as_deref();
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
                x: (input.x * self.cell.0 + self.cell.0 / 2) as f32,
                y: (input.y * self.cell.1 + self.cell.1 / 2) as f32,
            });
        self.mouse_encoder
            .set_options_from_terminal(&self.term)
            .set_size(mouse::EncoderSize {
                screen_width: u32::from(cols) * self.cell.0,
                screen_height: u32::from(rows) * self.cell.1,
                cell_width: self.cell.0,
                cell_height: self.cell.1,
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
        // Image placements are read in viewport coordinates: the viewport is
        // the screen from here on (the scroll position is not kept anyway).
        self.term.scroll_viewport(ScrollViewport::Bottom);

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
            let mut content =
                format_range(&self.term, Point::Active, (0, 0), (last.x, last.y), false)?;
            content.extend(kitty_images(&self.term, &mut self.placements)?);
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
        out.extend(kitty_images(&self.term, &mut self.placements)?);
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
    term.resize(cols, rows, DEFAULT_CELL_PX.0, DEFAULT_CELL_PX.1)?;
    // Keep multi-codepoint graphemes (ZWJ emoji, flags) in one cell. A full
    // reset (RIS) turns this back off; fixing that needs a patched
    // libghostty-vt.
    term.set_mode(Mode::GRAPHEME_CLUSTER, true)?;
    // Kitty graphics: images are kept (and placed) by libghostty once the
    // storage has a limit; PNG needs a decoder, installed per thread.
    graphics::set_png_decoder(Some(Box::new(PngDecoder)))?;
    term.set_kitty_image_storage_limit(KITTY_STORAGE_LIMIT)?;
    Ok(term)
}

/// A virtual placement in the storage: what the placeholder cells of its
/// image show a part of.
struct VirtualPlacement {
    image_id: u32,
    placement_id: u32,
    columns: u32,
    rows: u32,
    z: i32,
}

/// A run of placeholder cells on one row: `width` cells from column `x`,
/// showing image row `row` from image column `col` on.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlaceholderRun {
    x: u32,
    image_id: u32,
    placement_id: u32,
    row: u32,
    col: u32,
    width: u32,
}

/// One placeholder cell as written: what its diacritics and colors say,
/// with what they leave out (a cell may continue the run before it).
struct PlaceholderCell {
    x: u32,
    id_low: u32,
    id_high: Option<u32>,
    placement_id: u32,
    row: Option<u32>,
    col: Option<u32>,
}

/// Builds the runs of a row, cell by cell, as Kitty joins them: a cell with
/// no row and column, or with the ones that follow, continues the run.
#[derive(Default)]
struct PlaceholderRuns {
    runs: Vec<PlaceholderRun>,
    current: Option<(PlaceholderRun, Option<u32>)>,
}

impl PlaceholderRuns {
    fn push(&mut self, x: u32, cps: &[char], style: Option<VtStyle>) {
        let cell = PlaceholderCell {
            x,
            id_low: style.map_or(0, |s| color_id(s.fg_color)),
            id_high: cps.get(3).and_then(|&cp| diacritic_index(cp)),
            placement_id: style.map_or(0, |s| color_id(s.underline_color)),
            row: cps.get(1).and_then(|&cp| diacritic_index(cp)),
            col: cps.get(2).and_then(|&cp| diacritic_index(cp)),
        };
        if let Some((run, high)) = &mut self.current {
            let continues = run.image_id & 0xff_ffff == cell.id_low
                && run.placement_id == cell.placement_id
                && cell.row.is_none_or(|row| row == run.row)
                && cell.col.is_none_or(|col| col == run.col + run.width)
                && (cell.id_high.is_none() || cell.id_high == *high);
            if continues {
                run.width += 1;
                return;
            }
        }
        self.end();
        self.current = Some((
            PlaceholderRun {
                x: cell.x,
                image_id: cell.id_low | (cell.id_high.unwrap_or(0) << 24),
                placement_id: cell.placement_id,
                row: cell.row.unwrap_or(0),
                col: cell.col.unwrap_or(0),
                width: 1,
            },
            cell.id_high,
        ));
    }

    fn end(&mut self) {
        if let Some((run, _)) = self.current.take() {
            self.runs.push(run);
        }
    }

    fn finish(mut self) -> Vec<PlaceholderRun> {
        self.end();
        self.runs
    }
}

/// The 24 bits of a color as Kitty reads an image id out of it.
fn color_id(color: StyleColor) -> u32 {
    match color {
        StyleColor::None => 0,
        StyleColor::Palette(index) => u32::from(index.0),
        StyleColor::Rgb(rgb) => u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b),
    }
}

/// The row or column a placeholder diacritic stands for.
fn diacritic_index(cp: char) -> Option<u32> {
    DIACRITICS
        .binary_search(&(cp as u32))
        .ok()
        .map(|i| i as u32)
}

/// What a placeholder run shows: the source rectangle of the image, the size
/// to draw it at and its offset in the run's first cell. The image is fitted
/// into the virtual placement's grid keeping its aspect ratio, centered, and
/// the run takes its share of that; the math is Ghostty's. None when the run
/// falls on the padding around the image.
fn placeholder_geometry(
    run: &PlaceholderRun,
    grid: (u32, u32),
    image: (u32, u32),
    cell: (u32, u32),
) -> Option<PlaceholderPart> {
    let (img_w, img_h) = (f64::from(image.0), f64::from(image.1));
    let grid_px = (
        f64::from(grid.0) * f64::from(cell.0),
        f64::from(grid.1) * f64::from(cell.1),
    );
    // Fit the image into the grid: the scale, and the padding that centers it.
    let (scale, pad_x, pad_y) = if img_w * grid_px.1 > img_h * grid_px.0 {
        let scale = grid_px.0 / img_w.max(1.0);
        (scale, 0.0, (grid_px.1 - img_h * scale) / 2.0)
    } else {
        let scale = grid_px.1 / img_h.max(1.0);
        (scale, (grid_px.0 - img_w * scale) / 2.0, 0.0)
    };
    // The padded image, in image pixels.
    let (pad_x, pad_y) = (pad_x / scale, pad_y / scale);
    let (padded_w, padded_h) = (img_w + pad_x * 2.0, img_h + pad_y * 2.0);
    // The run's share of the padded image.
    let mut src_x = padded_w * (f64::from(run.col) / f64::from(grid.0));
    let mut src_y = padded_h * (f64::from(run.row) / f64::from(grid.1));
    let mut src_w = padded_w * (f64::from(run.width) / f64::from(grid.0));
    let mut src_h = padded_h / f64::from(grid.1);
    let mut dest_w = f64::from(run.width) * f64::from(cell.0);
    let mut dest_h = f64::from(cell.1);
    let (mut off_x, mut off_y) = (0.0, 0.0);
    // Take the padding out of the share, as the image has none.
    if src_y < pad_y {
        let pad = pad_y - src_y;
        src_h -= pad;
        off_y = pad;
        dest_h -= pad * scale;
        src_y = 0.0;
        if src_h > img_h {
            src_h = img_h;
            dest_h = img_h * scale;
        }
    } else if src_y + src_h > padded_h - pad_y {
        src_y -= pad_y;
        src_h = padded_h - pad_y - src_y - pad_y;
        dest_h = src_h * scale;
    } else {
        src_y -= pad_y;
    }
    if src_x < pad_x {
        let pad = pad_x - src_x;
        src_w -= pad;
        off_x = pad;
        dest_w -= pad * scale;
        src_x = 0.0;
        if src_w > img_w {
            src_w = img_w;
            dest_w = img_w * scale;
        }
    } else if src_x + src_w > padded_w - pad_x {
        src_x -= pad_x;
        src_w = padded_w - pad_x - src_x - pad_x;
        dest_w = src_w * scale;
    } else {
        src_x -= pad_x;
    }
    if src_w <= 0.0 || src_h <= 0.0 {
        return None;
    }
    let px = |v: f64| v.round().max(0.0) as u32;
    Some(PlaceholderPart {
        source: (px(src_x), px(src_y), px(src_w), px(src_h)),
        size: (px(dest_w), px(dest_h)),
        offset: (px(off_x * scale), px(off_y * scale)),
    })
}

struct PlaceholderPart {
    source: (u32, u32, u32, u32),
    size: (u32, u32),
    offset: (u32, u32),
}

/// The diacritics Kitty numbers rows and columns of placeholder cells with:
/// the index is the value. Sorted, from Kitty's rowcolumn-diacritics.txt.
const DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C,
    0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
    0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592,
    0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
    0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
    0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
    0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3,
    0x0816, 0x0817, 0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954,
    0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71,
    0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7,
    0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE,
    0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1,
    0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186,
    0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

/// The image under `key` encoded once: a new one goes to `new_images` too.
/// None for an image the encoder does not take.
fn encode_cached(
    images: &mut HashMap<(u32, u64), Image>,
    image: &graphics::Image<'_>,
    new_images: &mut Vec<Image>,
) -> Result<Option<(u32, u64)>> {
    let key = (image.id()?, image.generation()?);
    if let Entry::Vacant(entry) = images.entry(key) {
        let Some(png) = encode_png(image)? else {
            return Ok(None);
        };
        let encoded = Image {
            id: key.0,
            generation: key.1,
            width: image.width()?,
            height: image.height()?,
            png: png.into(),
        };
        new_images.push(entry.insert(encoded).clone());
    }
    Ok(Some(key))
}

/// An image from the Kitty graphics storage as PNG, or None for a kind the
/// encoder does not take (libghostty keeps what it decoded, so compressed
/// data does not reach here).
fn encode_png(image: &graphics::Image<'_>) -> Result<Option<Vec<u8>>> {
    let color = match image.format()? {
        graphics::ImageFormat::Rgb => png::ColorType::Rgb,
        graphics::ImageFormat::Rgba => png::ColorType::Rgba,
        graphics::ImageFormat::Gray => png::ColorType::Grayscale,
        graphics::ImageFormat::GrayAlpha => png::ColorType::GrayscaleAlpha,
        _ => return Ok(None),
    };
    if image.compression()? != graphics::Compression::None {
        return Ok(None);
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width()?, image.height()?);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().context("png header")?;
    writer
        .write_image_data(image.data()?)
        .context("png image data")?;
    writer.finish().context("png finish")?;
    Ok(Some(out))
}

/// Decodes the PNGs programs send over the Kitty graphics protocol into
/// RGBA8 for libghostty. (The crate's own `RustPngDecoder` has no constructor
/// in 0.2.2, and reads into an empty buffer.)
struct PngDecoder;

impl graphics::DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc libghostty_vt::alloc::Allocator<'_>,
        data: &[u8],
    ) -> Option<graphics::DecodedImage<'alloc>> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
        decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return None;
        }
        let mut bytes =
            libghostty_vt::alloc::Bytes::new_with_alloc(alloc, info.buffer_size()).ok()?;
        bytes.copy_from_slice(&buf[..info.buffer_size()]);
        Some(graphics::DecodedImage {
            width: info.width,
            height: info.height,
            data: bytes,
        })
    }
}

/// The Kitty graphics on the active screen as the commands that put them
/// there: each image transmitted once (`a=t`, its id kept, as PNG), then
/// every placement of it at its cell (`a=p`, the cursor left alone), or as
/// the virtual placement its placeholder cells refer to. A placement whose
/// top is scrolled into the history cannot be put back and is left out.
fn kitty_images(
    term: &Terminal<'_, '_>,
    iterator: &mut graphics::PlacementIterator<'_>,
) -> Result<Vec<u8>> {
    use std::fmt::Write;

    let graphics = term.kitty_graphics()?;
    let mut out = String::new();
    let mut transmitted = std::collections::HashSet::new();
    let mut placements = iterator.update(&graphics)?;
    while let Some(placement) = placements.next() {
        let id = placement.image_id()?;
        let Some(image) = graphics.image(id) else {
            continue;
        };
        let position = if placement.is_virtual()? {
            None
        } else {
            match placement.viewport_pos(&image, term)? {
                Some(position) if position.row >= 0 && position.col >= 0 => Some(position),
                _ => continue,
            }
        };
        if transmitted.insert(id) {
            let Some(png) = encode_png(&image)? else {
                continue;
            };
            let encoded = base64(&png);
            let mut chunks = encoded.as_bytes().chunks(4096).peekable();
            let mut first = true;
            while let Some(chunk) = chunks.next() {
                let more = u8::from(chunks.peek().is_some());
                if first {
                    write!(out, "\x1b_Ga=t,q=2,i={id},f=100,m={more};")?;
                    first = false;
                } else {
                    write!(out, "\x1b_Gm={more};")?;
                }
                out.push_str(std::str::from_utf8(chunk)?);
                out.push_str("\x1b\\");
            }
        }
        match position {
            Some(position) => write!(
                out,
                "\x1b[{};{}H\x1b_Ga=p,q=2,C=1,i={id}",
                position.row + 1,
                position.col + 1
            )?,
            None => write!(out, "\x1b_Ga=p,q=2,U=1,i={id}")?,
        }
        for (key, value) in [
            ("p", placement.placement_id()?),
            ("c", placement.columns()?),
            ("r", placement.rows()?),
            ("x", placement.source_x()?),
            ("y", placement.source_y()?),
            ("w", placement.source_width()?),
            ("h", placement.source_height()?),
            ("X", placement.x_offset()?),
            ("Y", placement.y_offset()?),
        ] {
            if value != 0 {
                write!(out, ",{key}={value}")?;
            }
        }
        let z = placement.z()?;
        if z != 0 {
            write!(out, ",z={z}")?;
        }
        out.push_str(";\x1b\\");
    }
    Ok(out.into_bytes())
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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

/// The physical key `code` names (W3C `KeyboardEvent.code`, see
/// `KeyInput.code`), with its character on a US layout: the keys that type
/// characters.
fn physical_key(code: &str) -> Option<(key::Key, char)> {
    use key::Key;

    if let Some(letter) = code.strip_prefix("Key")
        && let [byte @ b'A'..=b'Z'] = letter.as_bytes()
    {
        let key = Key::try_from(Key::A as u32 + u32::from(byte - b'A')).ok()?;
        return Some((key, char::from(byte.to_ascii_lowercase())));
    }
    if let Some(digit) = code.strip_prefix("Digit")
        && let [byte @ b'0'..=b'9'] = digit.as_bytes()
    {
        let key = Key::try_from(Key::Digit0 as u32 + u32::from(byte - b'0')).ok()?;
        return Some((key, char::from(*byte)));
    }
    Some(match code {
        "Backquote" => (Key::Backquote, '`'),
        "Backslash" => (Key::Backslash, '\\'),
        "BracketLeft" => (Key::BracketLeft, '['),
        "BracketRight" => (Key::BracketRight, ']'),
        "Comma" => (Key::Comma, ','),
        "Equal" => (Key::Equal, '='),
        "Minus" => (Key::Minus, '-'),
        "Period" => (Key::Period, '.'),
        "Quote" => (Key::Quote, '\''),
        "Semicolon" => (Key::Semicolon, ';'),
        "Slash" => (Key::Slash, '/'),
        "Space" => (Key::Space, ' '),
        _ => return None,
    })
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
            pane_id: String::new(),
            action: KeyAction::Press as i32,
            mods,
            named: named as i32,
            codepoint: codepoint as u32,
            text: text.to_owned(),
            code: String::new(),
        }
    }

    /// A key with the physical key it is (`KeyInput.code`).
    fn at(code: &str, codepoint: char, mods: u32, text: &str) -> KeyInput {
        KeyInput {
            code: code.to_owned(),
            ..key(NamedKey::Unspecified, codepoint, mods, text)
        }
    }

    fn encoded(screen: &mut GhosttyScreen, input: KeyInput) -> Vec<u8> {
        let mut out = Vec::new();
        screen.encode_key(&input, &mut out).unwrap();
        out
    }

    /// A 20x4 RGB image (f=24), red left half, blue right, placed over 10 columns and 2 rows.
    fn kitty_image(extra: &str) -> Vec<u8> {
        use std::fmt::Write;
        let mut pixels = Vec::new();
        for _y in 0..4 {
            for x in 0..20 {
                pixels.extend_from_slice(if x < 10 { &[255, 0, 0] } else { &[0, 0, 255] });
            }
        }
        let mut out = String::new();
        write!(
            out,
            "\x1b_Ga=T,f=24,s=20,v=4,c=10,r=2,i=7,q=2{extra};{}\x1b\\",
            base64(&pixels)
        )
        .unwrap();
        out.into_bytes()
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        let reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        (reader.info().width, reader.info().height)
    }

    #[test]
    fn kitty_image_comes_as_a_placement_and_a_png() {
        let mut screen = screen();
        screen.feed(b"ab");
        screen.feed(&kitty_image(""));
        let frame = screen.frame(true).unwrap().unwrap();
        assert!(frame.placements_changed);
        assert_eq!(frame.placements.len(), 1);
        let placement = &frame.placements[0];
        assert_eq!(
            (placement.col, placement.row, placement.cols, placement.rows),
            (2, 0, 10, 2)
        );
        assert_eq!(placement.image_id, 7);
        assert_eq!((placement.source_width, placement.source_height), (20, 4));
        assert_eq!((placement.pixel_width, placement.pixel_height), (100, 40));
        assert_eq!(frame.images.len(), 1);
        let image = &frame.images[0];
        assert_eq!(
            (image.id, image.generation),
            (7, placement.image_generation)
        );
        assert_eq!((image.width, image.height), (20, 4));
        assert_eq!(png_size(&image.png), (20, 4));
        // The cursor went past the image, as Kitty says.
        let cursor = frame.cursor.unwrap();
        assert_eq!((cursor.x, cursor.y), (12, 1));
    }

    #[test]
    fn placements_and_images_are_sent_only_when_they_change() {
        let mut screen = screen();
        screen.feed(&kitty_image(""));
        screen.frame(true).unwrap();
        assert!(screen.frame(false).unwrap().is_none());

        // Text under the image: the rows come, the placements do not.
        screen.feed(b"\x1b[4;1Hx");
        let frame = screen.frame(false).unwrap().unwrap();
        assert!(!frame.placements_changed);
        assert!(frame.placements.is_empty());
        assert!(frame.images.is_empty());

        // Scrolled by one: the placement's top is out of view, the image is known already.
        screen.feed(b"\r\n");
        let frame = screen.frame(false).unwrap().unwrap();
        assert!(frame.placements_changed);
        assert_eq!(frame.placements[0].row, -1);
        assert!(frame.images.is_empty());

        // A full frame (a new subscriber) carries the image again.
        let frame = screen.frame(true).unwrap().unwrap();
        assert_eq!(frame.images.len(), 1);
        assert_eq!(frame.placements.len(), 1);

        // Scrolled out: no placements, the image is forgotten until it is in view again.
        screen.feed(b"\r\n");
        let frame = screen.frame(false).unwrap().unwrap();
        assert!(frame.placements_changed);
        assert!(frame.placements.is_empty());
        assert!(screen.frame(true).unwrap().unwrap().images.is_empty());
        screen.scroll(-1);
        let frame = screen.frame(false).unwrap().unwrap();
        assert_eq!(frame.placements[0].row, -1);
        assert_eq!(frame.images.len(), 1);

        // Deleted: a frame with no placements, and nothing else in it.
        screen.scroll_to_bottom();
        screen.scroll(-1);
        screen.frame(false).unwrap();
        screen.feed(b"\x1b_Ga=d,d=A,q=2\x1b\\");
        let frame = screen.frame(false).unwrap().unwrap();
        assert!(frame.placements_changed);
        assert!(frame.placements.is_empty());
        assert!(frame.changed.is_empty());
    }

    /// `count` placeholder cells of image 7 on the current row, for image row `row`,
    /// columns from 0: the row and column in diacritics, the id in the foreground color.
    fn placeholders(row: usize, count: usize) -> Vec<u8> {
        let mut out = String::from("\x1b[38;5;7m");
        for &col in &DIACRITICS[..count] {
            out.push(PLACEHOLDER);
            out.push(char::from_u32(DIACRITICS[row]).unwrap());
            out.push(char::from_u32(col).unwrap());
        }
        out.push_str("\x1b[0m");
        out.into_bytes()
    }

    #[test]
    fn placeholder_cells_show_their_part_of_a_virtual_placement() {
        let mut screen = screen();
        // A 20x4 image in a virtual placement of 10x2 cells (100x40 px): fitted
        // by width, scale 5, 10 px of padding above and below.
        screen.feed(&kitty_image(",U=1"));
        screen.feed(&placeholders(0, 10));
        screen.feed(b"\r\n");
        screen.feed(&placeholders(1, 10));
        let frame = screen.frame(true).unwrap().unwrap();
        assert_eq!(
            row_text(&frame, 0).trim(),
            "",
            "placeholder cells are blank"
        );
        assert_eq!(frame.placements.len(), 2);
        let top = &frame.placements[0];
        assert_eq!((top.col, top.row, top.cols, top.rows), (0, 0, 10, 1));
        assert_eq!(
            (
                top.source_x,
                top.source_y,
                top.source_width,
                top.source_height
            ),
            (0, 0, 20, 2)
        );
        assert_eq!(
            (
                top.pixel_width,
                top.pixel_height,
                top.x_offset,
                top.y_offset
            ),
            (100, 10, 0, 10)
        );
        let bottom = &frame.placements[1];
        assert_eq!((bottom.col, bottom.row, bottom.cols), (0, 1, 10));
        assert_eq!(
            (bottom.source_y, bottom.source_height, bottom.y_offset),
            (2, 2, 0)
        );
        assert_eq!(frame.images.len(), 1);

        // Cells without diacritics continue the run before them; a gap ends it.
        screen.feed("\x1b[3;1H\x1b[38;5;7m\u{10EEEE}\u{0305}\u{0305}\u{10EEEE} \u{10EEEE}\u{0305}\u{030D}\x1b[0m".as_bytes());
        let frame = screen.frame(false).unwrap().unwrap();
        let third: Vec<_> = frame.placements.iter().filter(|p| p.row == 2).collect();
        assert_eq!(third.len(), 2);
        assert_eq!((third[0].col, third[0].cols), (0, 2));
        assert_eq!((third[1].col, third[1].cols), (3, 1));

        // The placeholders overwritten: their placements go.
        screen.feed(b"\x1b[2;1H\x1b[K");
        let frame = screen.frame(false).unwrap().unwrap();
        assert!(frame.placements_changed);
        assert!(frame.placements.iter().all(|p| p.row != 1));
    }

    #[test]
    fn placements_are_measured_in_the_client_cell() {
        let mut screen = screen();
        screen.resize(20, 4, (8, 16)).unwrap();
        screen.feed(&kitty_image(""));
        let frame = screen.frame(true).unwrap().unwrap();
        assert_eq!((frame.cell_width_px, frame.cell_height_px), (8, 16));
        let placement = &frame.placements[0];
        assert_eq!((placement.cols, placement.rows), (10, 2));
        assert_eq!((placement.pixel_width, placement.pixel_height), (80, 32));
        // Restored with that cell, the same.
        let snapshot = screen.snapshot().unwrap();
        let mut restored = GhosttyScreen::restore(20, 4, (8, 16), &snapshot).unwrap();
        let frame = restored.frame(true).unwrap().unwrap();
        assert_eq!(
            (frame.cell_width_px, frame.placements[0].pixel_width),
            (8, 80)
        );
    }

    #[test]
    fn snapshot_keeps_virtual_placements() {
        let mut bytes = kitty_image(",U=1");
        bytes.extend(placeholders(0, 10));
        bytes.extend(b"\r\n");
        bytes.extend(placeholders(1, 10));
        let mut restored = round_trip(&bytes);
        let frame = restored.frame(true).unwrap().unwrap();
        assert_eq!(frame.placements.len(), 2);
        assert_eq!(
            (frame.placements[0].cols, frame.placements[0].pixel_height),
            (10, 10)
        );
        assert_eq!(frame.images.len(), 1);
    }

    #[test]
    fn an_image_sent_again_gets_a_new_generation() {
        let mut screen = screen();
        screen.feed(&kitty_image(""));
        let first = screen.frame(true).unwrap().unwrap();
        screen.feed(b"\x1b[H");
        screen.feed(&kitty_image(""));
        let frame = screen.frame(false).unwrap().unwrap();
        assert_eq!(frame.images.len(), 1);
        assert!(frame.images[0].generation > first.images[0].generation);
        assert_eq!(
            frame.placements[0].image_generation,
            frame.images[0].generation
        );
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
        assert_eq!(frame.viewport_top, 4);
        assert_eq!(screen.scrolled_back(), 3);
        screen.scroll_to_bottom();
        let frame = screen.frame(false).unwrap().unwrap();
        assert_eq!(row_text(&frame, 0), "line7");
        assert_eq!(frame.viewport_top, 7);
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

    #[test]
    fn ctrl_and_alt_on_a_layout_that_is_not_latin_go_by_the_physical_key() {
        let mut screen = screen();
        let (ctrl, alt, shift) = (mods::CTRL, mods::ALT, mods::SHIFT);
        // Russian: the C key types "с", the Z key "я", the A key "ф".
        for text in ["\u{441}", ""] {
            assert_eq!(
                encoded(&mut screen, at("KeyC", '\u{441}', ctrl, text)),
                b"\x03",
                "{text:?}"
            );
        }
        assert_eq!(
            encoded(&mut screen, at("KeyZ", '\u{44f}', ctrl, "\u{44f}")),
            b"\x1a"
        );
        assert_eq!(
            encoded(&mut screen, at("KeyA", '\u{444}', alt, "\u{444}")),
            b"\x1ba"
        );
        assert_eq!(
            encoded(&mut screen, at("KeyA", '\u{444}', alt | shift, "\u{424}")),
            b"\x1bA"
        );
        // The ] key ("ъ"), as Ctrl+] on a US layout.
        assert_eq!(
            encoded(&mut screen, at("BracketRight", '\u{44a}', ctrl, "")),
            b"\x1d"
        );
        assert_eq!(
            encoded(&mut screen, at("Digit5", '5', ctrl | alt, "")),
            encoded(&mut screen, key(NamedKey::Unspecified, '5', ctrl | alt, ""))
        );
        // Greek: Ctrl+ψ is the C key.
        assert_eq!(
            encoded(&mut screen, at("KeyC", '\u{3c8}', ctrl, "\u{3c8}")),
            b"\x03"
        );
        // Typing goes by the text, as ever.
        assert_eq!(
            encoded(&mut screen, at("KeyC", '\u{441}', 0, "\u{441}")),
            "\u{441}".as_bytes()
        );
        assert_eq!(
            encoded(&mut screen, at("KeyC", '\u{441}', shift, "\u{421}")),
            "\u{421}".as_bytes()
        );
        // Without the physical key (an older client): nothing to go by.
        assert_eq!(
            encoded(&mut screen, key(NamedKey::Unspecified, '\u{441}', ctrl, "")),
            b""
        );
    }

    #[test]
    fn a_latin_character_wins_over_the_physical_key() {
        let mut screen = screen();
        let (ctrl, alt) = (mods::CTRL, mods::ALT);
        // Dvorak: "c" is on the I key; AZERTY: "a" is on the Q key, "w" on Z.
        assert_eq!(encoded(&mut screen, at("KeyI", 'c', ctrl, "")), b"\x03");
        assert_eq!(encoded(&mut screen, at("KeyI", 'c', ctrl, "c")), b"\x03");
        assert_eq!(encoded(&mut screen, at("KeyQ", 'a', ctrl, "a")), b"\x01");
        assert_eq!(encoded(&mut screen, at("KeyZ", 'w', ctrl, "")), b"\x17");
        assert_eq!(encoded(&mut screen, at("KeyQ", 'a', alt, "a")), b"\x1ba");
        assert_eq!(encoded(&mut screen, at("KeyI", 'c', 0, "c")), b"c");
    }

    #[test]
    fn the_kitty_protocol_gets_the_physical_key_as_the_base_layout_key() {
        let mut screen = screen();
        // Disambiguate, report alternate keys.
        screen.feed(b"\x1b[>5u");
        let ctrl = mods::CTRL;
        assert_eq!(
            encoded(&mut screen, at("KeyC", '\u{441}', ctrl, "\u{441}")),
            b"\x1b[1089::99;5u"
        );
        assert_eq!(
            encoded(&mut screen, at("KeyI", 'c', ctrl, "")),
            b"\x1b[99::105;5u"
        );
        assert_eq!(
            encoded(&mut screen, at("KeyC", 'c', ctrl, "")),
            b"\x1b[99;5u"
        );
    }

    /// Input is appended to a buffer that may still hold earlier, unsent
    /// input and have only a few spare bytes.
    #[test]
    fn encoding_appends_to_a_nearly_full_buffer() {
        let mut screen = screen();
        screen.feed(b"\x1b[?1000h\x1b[?1006h");
        let click = MouseInput {
            pane_id: String::new(),
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
            pane_id: String::new(),
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
        let mut restored = GhosttyScreen::restore(20, 4, DEFAULT_CELL_PX, &snapshot).unwrap();
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
    fn snapshot_keeps_images_on_both_screens() {
        let mut bytes = b"ab".to_vec();
        bytes.extend(kitty_image(",z=-1,X=3"));
        bytes.extend(b"\x1b[?1049h\x1b[2;2H");
        bytes.extend(kitty_image(",i=8,p=2"));
        let mut restored = round_trip(&bytes);
        let frame = restored.frame(true).unwrap().unwrap();
        assert!(frame.alt_screen);
        assert_eq!(frame.placements.len(), 1);
        assert_eq!(frame.placements[0].image_id, 8);
        assert_eq!((frame.placements[0].col, frame.placements[0].row), (1, 1));
        assert_eq!(frame.images.len(), 1);
        assert_eq!(png_size(&frame.images[0].png), (20, 4));

        restored.feed(b"\x1b[?1049l");
        let frame = restored.frame(true).unwrap().unwrap();
        assert!(!frame.alt_screen);
        let placement = &frame.placements[0];
        assert_eq!(
            (placement.image_id, placement.col, placement.row),
            (7, 2, 0)
        );
        assert_eq!(
            (
                placement.cols,
                placement.rows,
                placement.z,
                placement.x_offset
            ),
            (10, 2, -1, 3)
        );
        assert_eq!(row_text(&frame, 0), "ab");
    }

    #[test]
    fn snapshot_leaves_out_an_image_scrolled_into_the_history() {
        let mut bytes = b"ab".to_vec();
        bytes.extend(kitty_image(""));
        // Text on every row (the formatter drops blank history rows); the
        // image's top ends up one row up in the history.
        bytes.extend(b"c\r\nd\r\ne\r\nf\r\ng");
        let mut restored = round_trip(&bytes);
        let frame = restored.frame(true).unwrap().unwrap();
        assert!(frame.placements.is_empty());
        restored.scroll(-1);
        assert!(
            restored
                .frame(false)
                .unwrap()
                .unwrap()
                .placements
                .is_empty()
        );
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
