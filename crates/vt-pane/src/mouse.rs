//! Mouse reporting, wheel translation and text selection for a session.
//!
//! Pure functions over [`Session`]: encode pointer events with the ghostty
//! mouse encoder (modes auto-synced from the terminal), route the wheel the
//! way desktop terminals do (app reporting -> alternate-screen arrow keys ->
//! viewport scrollback) and drive the ghostty selection gesture machine
//! (click / double-click word / triple-click line / drag / rectangle).

use std::time::{Duration, Instant};

use anyhow::Result;
use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::key::{Key as GKey, Mods as GMods};
use libghostty_vt::mouse::{self, EncoderSize};
use libghostty_vt::screen::Screen;
use libghostty_vt::selection::gesture::{DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent};
use libghostty_vt::terminal::{Mode, Point, PointCoordinate, ScrollViewport};

use crate::task as vtask;
use crate::Session;

/// Multi-click window for double/triple clicks (desktop-terminal default).
const REPEAT_INTERVAL: Duration = Duration::from_millis(500);
/// Lines per wheel step for arrows/scrollback (GNOME Terminal default).
pub const WHEEL_STEP_LINES: usize = 3;

/// Reusable encoder/gesture objects owned by each session (plain data;
/// functions below take it via `&mut Session`).
pub struct PointerState {
    encoder: mouse::Encoder<'static>,
    event: mouse::Event<'static>,
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    drag: DragEvent<'static>,
    release: ReleaseEvent<'static>,
    t0: Instant,
    /// Bitset of the mouse DEC modes (tracking kind + output format) the
    /// encoder options derive from; the C setopt clears per-cell motion
    /// dedup, so only refresh when the snapshot changes.
    last_modes: Option<u16>,
    /// Cached encoder size (setopt also clears motion-dedup state).
    last_size: Option<(u32, u32, u32, u32)>,
}

pub fn new_pointer_state() -> Result<PointerState> {
    let mut press = PressEvent::new()?;
    press.set_repeat_interval(REPEAT_INTERVAL)?;
    Ok(PointerState {
        encoder: mouse::Encoder::new()?,
        event: mouse::Event::new()?,
        gesture: Gesture::new()?,
        press,
        drag: DragEvent::new()?,
        release: ReleaseEvent::new()?,
        t0: Instant::now(),
        last_modes: None,
        last_size: None,
    })
}

/// Any mouse tracking mode (9/1000/1002/1003) requested by the child.
pub fn is_mouse_tracking(sess: &Session) -> bool {
    sess.term.is_mouse_tracking().unwrap_or(false)
}

/// All DEC modes `set_options_from_terminal` derives encoder options
/// from — tracking kind (9/1000/1002/1003) and output format
/// (1005/1006/1015/1016) — packed into a bitset for cache comparison.
fn mouse_mode_bits(sess: &Session) -> u16 {
    const MODES: [Mode; 8] = [
        Mode::X10_MOUSE,
        Mode::NORMAL_MOUSE,
        Mode::BUTTON_MOUSE,
        Mode::ANY_MOUSE,
        Mode::UTF8_MOUSE,
        Mode::SGR_MOUSE,
        Mode::URXVT_MOUSE,
        Mode::SGR_PIXELS_MOUSE,
    ];
    let mut bits = 0u16;
    for (i, m) in MODES.iter().enumerate() {
        if sess.term.mode(*m).unwrap_or(false) {
            bits |= 1 << i;
        }
    }
    bits
}

/// Alternate screen active (vim/less/htop full-screen mode).
pub fn alt_screen(sess: &Session) -> bool {
    matches!(sess.term.active_screen(), Ok(Screen::Alternate))
}

/// What a wheel event should do (pure; unit-tested).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelRoute {
    /// Forward as button 4/5 reports to the child.
    Report,
    /// Send arrow keys (alternate screen, no tracking; VTE-style default).
    Arrows,
    /// Scroll the local viewport through scrollback.
    Viewport,
}

pub fn wheel_route(tracking: bool, shift: bool, alt: bool) -> WheelRoute {
    if tracking && !shift {
        WheelRoute::Report
    } else if alt && !shift {
        WheelRoute::Arrows
    } else {
        WheelRoute::Viewport
    }
}

/// Arrow key and repeat count for a wheel delta in lines (up = positive).
pub fn wheel_arrows(lines: f32) -> Option<(GKey, usize)> {
    if lines.abs() < f32::EPSILON {
        return None;
    }
    let key = if lines > 0.0 {
        GKey::ArrowUp
    } else {
        GKey::ArrowDown
    };
    let steps = (lines.abs().round().max(1.0) as usize).saturating_mul(WHEEL_STEP_LINES);
    Some((key, steps))
}

/// Viewport scroll delta for a wheel delta in lines (up = negative).
pub fn wheel_delta(lines: f32) -> Option<isize> {
    if lines.abs() < f32::EPSILON {
        return None;
    }
    let steps = lines.abs().round().max(1.0) as isize;
    Some(if lines > 0.0 { -steps } else { steps } * WHEEL_STEP_LINES as isize)
}

/// Clamp surface-space pointer pixels into the terminal grid.
fn viewport_point(sess: &Session, x: f32, y: f32) -> Point {
    let (cw, ch) = vtask::cell_px(sess);
    let cols = f32::from(sess.term.cols().unwrap_or(80));
    let rows = f32::from(sess.term.rows().unwrap_or(24));
    Point::Viewport(PointCoordinate {
        x: (x / cw.max(1) as f32).clamp(0.0, cols) as u16,
        y: (y / ch.max(1) as f32).clamp(0.0, rows) as u32,
    })
}

/// Clamp surface pixels to the actual grid extent (the pane rect can be
/// taller/wider than whole cells; the trailing sliver is dead space).
fn clamp_grid_px(sess: &Session, x: f32, y: f32) -> (f32, f32) {
    let (cw, ch) = vtask::cell_px(sess);
    let (cw, ch) = (cw.max(1) as f32, ch.max(1) as f32);
    let cols = f32::from(sess.term.cols().unwrap_or(80));
    let rows = f32::from(sess.term.rows().unwrap_or(24));
    (x.clamp(0.0, cols * cw - 1.0), y.clamp(0.0, rows * ch - 1.0))
}

fn encoder_size(sess: &Session) -> EncoderSize {
    let (cw, ch) = vtask::cell_px(sess);
    let (cw, ch) = (cw.max(1), ch.max(1));
    EncoderSize {
        screen_width: u32::from(sess.term.cols().unwrap_or(80)).saturating_mul(cw),
        screen_height: u32::from(sess.term.rows().unwrap_or(24)).saturating_mul(ch),
        cell_width: cw,
        cell_height: ch,
        padding_top: 0,
        padding_bottom: 0,
        padding_right: 0,
        padding_left: 0,
    }
}

/// Encode one pointer event against the terminal's live mouse modes.
/// Surface-pixel coordinates are relative to the pane's grid origin.
/// Returns the pty bytes without writing (testable without a live child).
pub fn encode_mouse(
    sess: &mut Session,
    action: mouse::Action,
    button: Option<mouse::Button>,
    mods: GMods,
    x: f32,
    y: f32,
    any_button: bool,
) -> Result<Vec<u8>> {
    let size = encoder_size(sess);
    let key = (
        size.screen_width,
        size.screen_height,
        size.cell_width,
        size.cell_height,
    );
    let modes = mouse_mode_bits(sess);
    let stale = sess.pointer.last_modes != Some(modes);
    let resized = sess.pointer.last_size != Some(key);
    let (x, y) = clamp_grid_px(sess, x, y);
    let PointerState {
        encoder,
        event,
        last_modes,
        last_size,
        ..
    } = &mut sess.pointer;
    event.set_action(action);
    event.set_button(button);
    event.set_mods(mods);
    event.set_position(mouse::Position { x, y });
    if stale {
        encoder.set_options_from_terminal(&sess.term);
        *last_modes = Some(modes);
    }
    if resized {
        encoder.set_size(size);
        *last_size = Some(key);
    }
    encoder
        .set_any_button_pressed(any_button)
        .set_track_last_cell(true);
    let mut out = Vec::with_capacity(32);
    encoder.encode_to_vec(event, &mut out)?;
    Ok(out)
}

/// Forward one pointer event to the child (a no-op encode when the
/// terminal modes say the app does not want this event).
pub fn send_mouse(
    sess: &mut Session,
    action: mouse::Action,
    button: Option<mouse::Button>,
    mods: GMods,
    x: f32,
    y: f32,
    any_button: bool,
) -> Result<()> {
    let out = encode_mouse(sess, action, button, mods, x, y, any_button)?;
    if !out.is_empty() {
        vtask::write(sess, &out)?;
    }
    Ok(())
}

/// Route one wheel event (delta in lines, up = positive; surface-pixel pos).
pub fn send_wheel(
    sess: &mut Session,
    lines: f32,
    mods: GMods,
    x: f32,
    y: f32,
    any_button: bool,
) -> Result<()> {
    let route = wheel_route(
        is_mouse_tracking(sess),
        mods.intersects(GMods::SHIFT),
        alt_screen(sess),
    );
    match route {
        WheelRoute::Report => {
            let button = if lines > 0.0 {
                mouse::Button::Four
            } else {
                mouse::Button::Five
            };
            for _ in 0..lines.abs().round().max(1.0) as usize {
                send_mouse(
                    sess,
                    mouse::Action::Press,
                    Some(button),
                    mods,
                    x,
                    y,
                    any_button,
                )?;
            }
        }
        WheelRoute::Arrows => {
            if let Some((key, steps)) = wheel_arrows(lines) {
                for _ in 0..steps {
                    vtask::send_key(sess, |ev| {
                        ev.set_key(key);
                    })?;
                }
            }
        }
        WheelRoute::Viewport => {
            if let Some(delta) = wheel_delta(lines) {
                sess.term.scroll_viewport(ScrollViewport::Delta(delta));
            }
        }
    }
    Ok(())
}

/// Clear the active selection (mouse-grabbing apps own the screen).
pub fn clear_selection(sess: &mut Session) {
    let _ = sess.term.set_selection(None);
}

/// Snap the viewport back to the live area if the user had scrolled up.
pub fn follow_output(sess: &mut Session) {
    if !sess.term.viewport_active().unwrap_or(true) {
        sess.term.scroll_viewport(ScrollViewport::Bottom);
    }
}

/// Press: starts/restarts a selection gesture (multi-click aware).
pub fn select_press(sess: &mut Session, x: f32, y: f32) -> Result<()> {
    let (x, y) = clamp_grid_px(sess, x, y);
    let grid_ref = sess.term.grid_ref(viewport_point(sess, x, y))?;
    let (cw, _) = vtask::cell_px(sess);
    let elapsed = sess.pointer.t0.elapsed();
    let PointerState { press, gesture, .. } = &mut sess.pointer;
    let sel = press
        .set_repeat_distance(f64::from(cw.max(1)))?
        .set_time(elapsed)?
        .set_position(f64::from(x), f64::from(y))?
        .apply(gesture, &sess.term, grid_ref)?;
    sess.term.set_selection(sel.as_ref())?;
    Ok(())
}

/// Drag: extends the active selection (rectangle for Ctrl+Alt drags).
pub fn select_drag(sess: &mut Session, x: f32, y: f32, rectangle: bool) -> Result<()> {
    let (x, y) = clamp_grid_px(sess, x, y);
    let grid_ref = sess.term.grid_ref(viewport_point(sess, x, y))?;
    let geometry = {
        let (cw, ch) = vtask::cell_px(sess);
        Geometry {
            columns: u32::from(sess.term.cols().unwrap_or(80)),
            cell_width: cw.max(1),
            padding_left: 0,
            screen_height: u32::from(sess.term.rows().unwrap_or(24)).saturating_mul(ch.max(1)),
        }
    };
    let sel = sess
        .pointer
        .drag
        .set_position(f64::from(x), f64::from(y))?
        .set_rectangle(rectangle)?
        .apply(&mut sess.pointer.gesture, &sess.term, grid_ref, geometry)?;
    sess.term.set_selection(sel.as_ref())?;
    Ok(())
}

/// Release: finishes the gesture; keeps the final selection installed.
pub fn select_release(sess: &mut Session, x: f32, y: f32) -> Result<()> {
    let (x, y) = clamp_grid_px(sess, x, y);
    let grid_ref = sess.term.grid_ref(viewport_point(sess, x, y)).ok();
    sess.pointer
        .release
        .apply(&mut sess.pointer.gesture, &sess.term, grid_ref)?;
    Ok(())
}

/// Plain text of the active selection ("" when none); unwrapped and
/// right-trimmed, ready for the clipboard.
pub fn selection_text(sess: &mut Session) -> Result<String> {
    let Some(sel) = sess.term.selection()? else {
        return Ok(String::new());
    };
    let opts = FormatterOptions::new()
        .with_format(Format::Plain)
        .with_unwrap(true)
        .with_trim(true)
        .with_selection(&sel);
    let mut fmt = Formatter::new(&sess.term, opts)?;
    let bytes = fmt.format_alloc(None)?;
    Ok(String::from_utf8_lossy(bytes.as_ref()).into_owned())
}
