//! Raw pointer-event routing for terminal panes: mouse-report forwarding,
//! drag selection and wheel translation (scrollback / arrows / reports).
//!
//! Runs on the raw egui event stream (before widget consumption) so that
//! terminal-grabbing apps receive clicks/drags/wheel exactly as they would
//! in a desktop terminal. Shift is the escape hatch: it always selects
//! locally and never reports.

use egui::{Context, Event, Pos2, PointerButton, Rect};
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
    rects
        .iter()
        .find(|(_, r)| r.contains(pos))
        .map(|(p, _)| *p)
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

/// Motion while a primary drag is active: extend the selection or report
/// button-motion to the child (position clamped to the owning pane).
fn on_motion(
    sess: &mut Session,
    rect: Rect,
    pos: Pos2,
    ppp: f32,
    mods: &egui::Modifiers,
) {
    let (x, y) = surface_px(rect, pos, ppp);
    if vmouse::is_mouse_tracking(sess) && !mods.shift {
        if let Err(e) = vmouse::send_mouse(
            sess,
            mouse::Action::Motion,
            Some(mouse::Button::Left),
            gmods(mods),
            x,
            y,
            true,
        ) {
            warn!("motion report: {e}");
        }
    } else if let Err(e) = vmouse::select_drag(sess, x, y, mods.alt && (mods.ctrl || mods.command))
    {
        warn!("selection drag: {e}");
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
                    continue;
                };
                match sess.map.get_mut(&pane) {
                    Some(s) if s.exit.is_none() => {
                        on_button(s, *rect, pos, ppp, button, pressed, &modifiers);
                        if button == PointerButton::Primary {
                            uist.pointer_pane = pressed.then_some(pane);
                        }
                    }
                    _ if !pressed => uist.pointer_pane = None,
                    _ => {}
                }
            }
            Event::PointerMoved(pos) => {
                let Some(pane) = uist.pointer_pane else {
                    continue;
                };
                let Some((_, rect)) = rects.iter().find(|(p, _)| *p == pane) else {
                    continue;
                };
                if let Some(s) = sess.map.get_mut(&pane) {
                    if s.exit.is_none() {
                        on_motion(s, *rect, pos, ppp, &mods_now);
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
                let target = uist.pointer_pane.or_else(|| hover.and_then(|p| pane_at(rects, p)));
                let Some(pane) = target else {
                    continue;
                };
                let lines = wheel_lines(unit, delta, cell_h);
                if lines == 0.0 {
                    continue;
                }
                let Some((_, rect)) = rects.iter().find(|(p, _)| *p == pane) else {
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

