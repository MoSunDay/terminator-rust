//! Raw pointer-event routing for terminal panes: mouse-report forwarding,
//! drag selection and wheel translation (scrollback / arrows / reports).
//!
//! Runs on the raw egui event stream (before widget consumption) so that
//! terminal-grabbing apps receive clicks/drags/wheel exactly as they would
//! in a desktop terminal. Shift is the escape hatch: it always selects
//! locally and never reports.

use egui::{Context, Event, PointerButton, Pos2, Rect};
use layout_tree::PaneId;
use libghostty_vt::key::Mods as GMods;
use libghostty_vt::mouse;
use log::warn;
use vt_pane::{mouse as vmouse, Session};

use crate::session_map::SessionMap;
use crate::state::{AppState, UiState};

/// egui modifiers -> ghostty mods.
fn gmods(m: &egui::Modifiers) -> GMods {
    let mut g = GMods::empty();
    if m.shift {
        g |= GMods::SHIFT;
    }
    if m.ctrl {
        g |= GMods::CTRL;
    }
    if m.alt {
        g |= GMods::ALT;
    }
    if m.command {
        g |= GMods::SUPER;
    }
    g
}

/// egui button -> ghostty mouse button (back/forward -> extended 8/9,
/// matching desktop-terminal SGR behavior).
fn gbutton(b: PointerButton) -> mouse::Button {
    match b {
        PointerButton::Primary => mouse::Button::Left,
        PointerButton::Secondary => mouse::Button::Right,
        PointerButton::Middle => mouse::Button::Middle,
        PointerButton::Extra1 => mouse::Button::Eight,
        PointerButton::Extra2 => mouse::Button::Nine,
    }
}

/// Grab bitmask bit for an egui pointer button.
fn button_bit(b: PointerButton) -> u8 {
    match b {
        PointerButton::Primary => 1 << 0,
        PointerButton::Secondary => 1 << 1,
        PointerButton::Middle => 1 << 2,
        PointerButton::Extra1 => 1 << 3,
        PointerButton::Extra2 => 1 << 4,
    }
}

/// egui button for a grab bitmask bit index (inverse of `button_bit`).
fn button_at(bit: u8) -> Option<PointerButton> {
    Some(match bit {
        0 => PointerButton::Primary,
        1 => PointerButton::Secondary,
        2 => PointerButton::Middle,
        3 => PointerButton::Extra1,
        4 => PointerButton::Extra2,
        _ => return None,
    })
}

/// Wheel vertical delta in lines (up = positive); 0 for horizontal-only.
fn wheel_lines(unit: egui::MouseWheelUnit, delta: egui::Vec2, cell_h: f32) -> f32 {
    match unit {
        egui::MouseWheelUnit::Line => delta.y,
        egui::MouseWheelUnit::Page => delta.y * 24.0,
        egui::MouseWheelUnit::Point => delta.y / cell_h.max(1.0),
    }
}

/// Pane whose content rect contains `pos` (headers/dividers excluded).
fn pane_at(rects: &[(PaneId, Rect)], pos: Pos2) -> Option<PaneId> {
    rects.iter().find(|(_, r)| r.contains(pos)).map(|(p, _)| *p)
}

/// Clamp `pos` into `rect` and return surface (physical) pixels from the
/// grid origin, scaled by `ppp` (the vt engine consumes physical pixels).
fn surface_px(rect: Rect, pos: Pos2, ppp: f32) -> (f32, f32) {
    let x = pos.x.clamp(rect.min.x, rect.max.x) - rect.min.x;
    let y = pos.y.clamp(rect.min.y, rect.max.y) - rect.min.y;
    (x * ppp, y * ppp)
}

/// Focus `pane` if it is not focused yet (persisted -> dirty).
fn focus_pane(st: &mut AppState, pane: PaneId, dirty: &mut bool) {
    if let Some(t) = st.tree.tabs.get_mut(st.tree.active_tab) {
        if t.focused != pane {
            t.focused = pane;
            *dirty = true;
        }
    }
}

/// Press/release inside `pane`: report to the child when it grabs the mouse,
/// otherwise drive the local selection gesture (primary button only).
fn on_button(
    sess: &mut Session,
    rect: Rect,
    pos: Pos2,
    ppp: f32,
    button: PointerButton,
    pressed: bool,
    mods: &egui::Modifiers,
) {
    let (x, y) = surface_px(rect, pos, ppp);
    let tracking = vmouse::is_mouse_tracking(sess);
    if tracking && !mods.shift {
        if pressed {
            vmouse::clear_selection(sess);
        }
        if let Err(e) = vmouse::send_mouse(
            sess,
            if pressed {
                mouse::Action::Press
            } else {
                mouse::Action::Release
            },
            Some(gbutton(button)),
            gmods(mods),
            x,
            y,
            false,
        ) {
            warn!("mouse report: {e}");
        }
        return;
    }
    // No reporting: primary drives selection; right stays with the
    // context menu; middle paste is deferred.
    if button == PointerButton::Primary {
        let res = if pressed {
            vmouse::select_press(sess, x, y)
        } else {
            vmouse::select_release(sess, x, y)
        };
        if let Err(e) = res {
            warn!("selection: {e}");
        }
    }
}

/// Motion while a grab of `button` is active: report button-motion to the
/// child when it tracks the mouse, else extend the selection drag
/// (primary button only; other buttons do nothing without tracking).
/// Position is clamped to the owning pane.
fn on_motion(
    sess: &mut Session,
    rect: Rect,
    pos: Pos2,
    ppp: f32,
    mods: &egui::Modifiers,
    button: PointerButton,
) {
    let (x, y) = surface_px(rect, pos, ppp);
    if vmouse::is_mouse_tracking(sess) && !mods.shift {
        if let Err(e) = vmouse::send_mouse(
            sess,
            mouse::Action::Motion,
            Some(gbutton(button)),
            gmods(mods),
            x,
            y,
            true,
        ) {
            warn!("motion report: {e}");
        }
    } else if button == PointerButton::Primary {
        if let Err(e) = vmouse::select_drag(sess, x, y, mods.alt && (mods.ctrl || mods.command)) {
            warn!("selection drag: {e}");
        }
    }
}

/// Wheel over `pane`: app reports, alternate-screen arrows or scrollback.
fn on_wheel(
    sess: &mut Session,
    rect: Rect,
    pos: Pos2,
    ppp: f32,
    lines: f32,
    mods: &egui::Modifiers,
    any_button: bool,
) {
    let (x, y) = surface_px(rect, pos, ppp);
    if let Err(e) = vmouse::send_wheel(sess, lines, gmods(mods), x, y, any_button) {
        warn!("wheel pane: {e}");
    }
}

/// One frame of raw pointer routing over the pane content rects.
/// `dirty` marks persisted state changes (focus moves).
pub fn handle(
    ctx: &Context,
    rects: &[(PaneId, Rect)],
    st: &mut AppState,
    sess: &mut SessionMap,
    uist: &mut UiState,
    cell_h: f32,
    dirty: &mut bool,
) {
    let events = ctx.input(|i| i.events.clone());
    let any_down = ctx.input(|i| i.pointer.any_down());
    let hover = ctx.input(|i| i.pointer.hover_pos());
    let mods_now = ctx.input(|i| i.modifiers);
    let ppp = ctx.pixels_per_point();
    for ev in events {
        match ev {
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } => {
                // Releases follow the press owner (implicit grab): a drag can
                // end outside its pane, and the owner must still see the
                // button go up.
                let pane = if pressed {
                    pane_at(rects, pos)
                } else {
                    uist.pointer_pane.or_else(|| pane_at(rects, pos))
                };
                let Some(pane) = pane else {
                    // Unreachable with an owner set (releases resolve to it),
                    // so there is nothing to clean up here.
                    continue;
                };
                if pressed {
                    focus_pane(st, pane, dirty);
                }
                let Some((_, rect)) = rects.iter().find(|(p, _)| *p == pane) else {
                    if uist.pointer_pane == Some(pane) {
                        // Owner vanished mid-grab (tab switch / relayout):
                        // drop the grab so motion/wheel stop targeting a
                        // zombie pane.
                        uist.pointer_pane = None;
                        uist.pointer_buttons = 0;
                        uist.pointer_last = None;
                    }
                    continue;
                };
                let bit = button_bit(button);
                match sess.map.get_mut(&pane) {
                    Some(s) if s.exit.is_none() => {
                        on_button(s, *rect, pos, ppp, button, pressed, &modifiers);
                        if pressed {
                            uist.pointer_pane = Some(pane);
                            uist.pointer_buttons |= bit;
                            uist.pointer_last = Some(bit.trailing_zeros() as u8);
                        } else {
                            uist.pointer_buttons &= !bit;
                            if uist.pointer_buttons == 0 {
                                uist.pointer_pane = None;
                                uist.pointer_last = None;
                            } else {
                                // Keep "most recent press still held" honest
                                // when a chorded button goes up first.
                                uist.pointer_last =
                                    Some(uist.pointer_buttons.trailing_zeros() as u8);
                            }
                        }
                    }
                    _ if !pressed => {
                        uist.pointer_pane = None;
                        uist.pointer_buttons = 0;
                        uist.pointer_last = None;
                    }
                    _ => {}
                }
            }
            Event::PointerMoved(pos) => {
                let Some(pane) = uist.pointer_pane else {
                    continue;
                };
                let Some((_, rect)) = rects.iter().find(|(p, _)| *p == pane) else {
                    if uist.pointer_pane == Some(pane) {
                        uist.pointer_pane = None;
                        uist.pointer_buttons = 0;
                        uist.pointer_last = None;
                    }
                    continue;
                };
                // Motion reports the most recent button still held; without
                // one there is no grab to service.
                let Some(bit) = uist.pointer_last else {
                    continue;
                };
                let Some(button) = button_at(bit) else {
                    continue;
                };
                if let Some(s) = sess.map.get_mut(&pane) {
                    if s.exit.is_none() {
                        on_motion(s, *rect, pos, ppp, &mods_now, button);
                    }
                }
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } => {
                // Wheel targets the pane under the pointer (hover), or the
                // drag owner while a drag is active.
                let target = uist
                    .pointer_pane
                    .or_else(|| hover.and_then(|p| pane_at(rects, p)));
                let Some(pane) = target else {
                    continue;
                };
                let lines = wheel_lines(unit, delta, cell_h);
                if lines == 0.0 {
                    continue;
                }
                let Some((_, rect)) = rects.iter().find(|(p, _)| *p == pane) else {
                    if uist.pointer_pane == Some(pane) {
                        uist.pointer_pane = None;
                        uist.pointer_buttons = 0;
                        uist.pointer_last = None;
                    }
                    continue;
                };
                let Some(pos) = hover else {
                    continue;
                };
                if let Some(s) = sess.map.get_mut(&pane) {
                    if s.exit.is_none() {
                        on_wheel(s, *rect, pos, ppp, lines, &modifiers, any_down);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_map::session_map;
    use crate::state::{fresh_state, ui_state};
    use egui::{Modifiers, Pos2, RawInput};
    use vt_pane::{task as vtask, SessionOpts};

    #[test]
    fn button_bit_round_trip() {
        for b in [
            PointerButton::Primary,
            PointerButton::Secondary,
            PointerButton::Middle,
            PointerButton::Extra1,
            PointerButton::Extra2,
        ] {
            assert_eq!(button_at(button_bit(b).trailing_zeros() as u8), Some(b));
        }
    }

    /// Live `cat` session under pane 1; the grab lifecycle needs a live
    /// session so the press arm runs. Skips when no pty is available.
    fn pty_harness() -> Option<SessionMap> {
        let opts = SessionOpts::command(
            20,
            5,
            vec!["sh".to_string(), "-c".to_string(), "cat".to_string()],
        );
        match vtask::spawn_session(&opts) {
            Ok(s) => {
                let mut m = session_map();
                m.map.insert(1, s);
                Some(m)
            }
            Err(e) => {
                eprintln!("skip: {e}");
                None
            }
        }
    }

    /// One synthetic egui pass carrying `ev`, then pointer dispatch over
    /// `rects` (same first-pass workaround as keyboard.rs tests).
    fn dispatch(
        ctx: &Context,
        ev: Event,
        rects: &[(PaneId, Rect)],
        st: &mut AppState,
        sess: &mut SessionMap,
        ui: &mut UiState,
    ) {
        let input = RawInput {
            events: vec![ev],
            ..Default::default()
        };
        ctx.begin_pass(input);
        let mut dirty = false;
        handle(ctx, rects, st, sess, ui, 16.0, &mut dirty);
        let mut out = ctx.end_pass();
        out.textures_delta.clear();
    }

    fn button_ev(button: PointerButton, pressed: bool, pos: Pos2) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn vanished_grab_owner_is_dropped() {
        let Some(mut sess) = pty_harness() else {
            return;
        };
        let (mut st, mut ui) = (fresh_state(), ui_state());
        let ctx = Context::default();
        let live = vec![(1, Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0)))];
        dispatch(
            &ctx,
            button_ev(PointerButton::Primary, true, Pos2::new(10.0, 10.0)),
            &live,
            &mut st,
            &mut sess,
            &mut ui,
        );
        assert_eq!(ui.pointer_pane, Some(1));
        assert_eq!(ui.pointer_buttons, 1);
        // Pane 1 vanishes mid-grab (tab switch / close). The stale owner
        // must be dropped instead of hijacking later motion and wheel.
        dispatch(
            &ctx,
            button_ev(PointerButton::Primary, false, Pos2::new(500.0, 500.0)),
            &[],
            &mut st,
            &mut sess,
            &mut ui,
        );
        assert_eq!(ui.pointer_pane, None);
        assert_eq!(ui.pointer_buttons, 0);
        assert_eq!(ui.pointer_last, None);
    }

    #[test]
    fn chorded_buttons_hold_grab_until_last_release() {
        let Some(mut sess) = pty_harness() else {
            return;
        };
        let (mut st, mut ui) = (fresh_state(), ui_state());
        let ctx = Context::default();
        let live = vec![(1, Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0)))];
        dispatch(
            &ctx,
            button_ev(PointerButton::Primary, true, Pos2::new(10.0, 10.0)),
            &live,
            &mut st,
            &mut sess,
            &mut ui,
        );
        dispatch(
            &ctx,
            button_ev(PointerButton::Secondary, true, Pos2::new(12.0, 12.0)),
            &live,
            &mut st,
            &mut sess,
            &mut ui,
        );
        assert_eq!(ui.pointer_pane, Some(1));
        assert_eq!(ui.pointer_buttons, 0b11);
        assert_eq!(ui.pointer_last, Some(1));
        // Releasing one chorded button keeps the grab for the other and
        // demotes the motion button to a button still held.
        dispatch(
            &ctx,
            button_ev(PointerButton::Secondary, false, Pos2::new(500.0, 500.0)),
            &live,
            &mut st,
            &mut sess,
            &mut ui,
        );
        assert_eq!(ui.pointer_pane, Some(1));
        assert_eq!(ui.pointer_buttons, 1);
        assert_eq!(ui.pointer_last, Some(0));
        dispatch(
            &ctx,
            button_ev(PointerButton::Primary, false, Pos2::new(500.0, 500.0)),
            &live,
            &mut st,
            &mut sess,
            &mut ui,
        );
        assert_eq!(ui.pointer_pane, None);
        assert_eq!(ui.pointer_buttons, 0);
        assert_eq!(ui.pointer_last, None);
    }
}
