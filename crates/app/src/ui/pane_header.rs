//! Per-pane header strip: title, rename, badges, close button and the
//! pane context menu.

use std::collections::BTreeMap;

use egui::{Button, Id, Key, Rect, Sense, TextEdit, Ui, Vec2};
use layout_tree::PaneId;
use theme::Palette;

use crate::actions;
use crate::render::colors::{self, to_c32};
use crate::session_map::SessionMap;
use crate::state::{
    effective_title, AppState, PaneAction, PaneDrag, PaneMeta, UiState, WindowState,
};

const BTN: f32 = 16.0;

/// None when the candidate manual title is acceptable: names must be
/// unique (control-socket addressing) and not digits-only (reserved for
/// pane ids in ctl's untagged PaneSelector).
fn reject_reason(
    panes: &BTreeMap<PaneId, PaneMeta>,
    value: &str,
    pane: PaneId,
) -> Option<&'static str> {
    if value.is_empty() {
        return None;
    }
    if value.chars().all(|c| c.is_ascii_digit()) {
        return Some("digits-only names are reserved for pane ids");
    }
    if crate::state::manual_title_taken(panes, value, pane) {
        return Some("another pane already uses this name");
    }
    None
}

/// Draw the header for `pane` in `rect` (full width, PANE_HEADER_H tall).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    pane: PaneId,
    tab: usize,
    st: &mut AppState,
    sess: &mut SessionMap,
    uist: &mut UiState,
    pal: &Palette,
    dirty: &mut bool,
) {
    let focused = st
        .win()
        .and_then(|w| w.tree.tabs.get(tab))
        .is_some_and(|t| t.focused == pane);
    let meta = st.panes.get(&pane);
    let osc = crate::session_map::osc_title(sess, pane);
    let exit = crate::session_map::exit_code(sess, pane);
    let title = match meta {
        Some(m) => effective_title(m.manual_title.as_deref(), &osc, &m.kind),
        None => "?".to_string(),
    };
    let degraded = meta.is_some_and(|m| m.degraded);

    // The header strip doubles as a drag handle (the native title bar is
    // the primary one; this stays as a convenience). Registered first so
    // the title/buttons on top keep their clicks (egui hit-test prefers
    // the topmost widget; drags fall through to this background).
    let drag = ui.interact(rect, Id::new("pane_header_drag").with(pane), Sense::drag());
    // A primary press on the header becomes a pane MOVE (drop target
    // tracked per frame in screen(); dropping on a sibling edge flips
    // the split axis) whenever the tab has another pane to rearrange
    // against or Ctrl forces it; a lone pane has nothing to rearrange,
    // so its header keeps the OS-window drag. Never while a pane drag
    // already owns the pointer.
    let pane_dragging = st
        .win()
        .is_some_and(|w| w.ui.pane_drag.is_some_and(|pd| pd.pane == pane));
    let tab_panes = st
        .win()
        .and_then(|w| w.tree.tabs.get(tab))
        .map(|t| layout_tree::pane_count(&t.root))
        .unwrap_or(1);
    let rearranges = header_starts_pane_move(ui.input(|i| i.modifiers.ctrl), tab_panes);
    if drag.drag_started_by(egui::PointerButton::Primary)
        && st.win().is_none_or(|w| w.ui.pane_drag.is_none())
    {
        if rearranges {
            if let Some(w) = st.win_mut() {
                w.ui.pane_drag = Some(PaneDrag { pane, target: None });
                w.ui.zoom = false;
            }
        } else {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    }
    if pane_dragging {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if drag.hovered() && rearranges {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // Quiet chrome: the header is a flat chrome strip (the focused pane is
    // already framed by its accent stroke) with a hairline over the
    // content; focus reads through the title color AND a faint accent
    // tint on the fill. Top corners round to meet the pane card stroke.
    let header_fill = if focused {
        colors::focus_header(pal)
    } else {
        colors::chrome_bg(pal)
    };
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: crate::render::tokens::R_MD,
            ne: crate::render::tokens::R_MD,
            sw: 0,
            se: 0,
        },
        colors::with_opacity(to_c32(header_fill), st.settings.opacity),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(
            1.0,
            colors::with_opacity(to_c32(colors::hairline(pal)), st.settings.opacity),
        ),
    );

    // Badges (right of the title area, before the buttons).
    let mut badges: Vec<String> = Vec::new();
    if degraded {
        badges.push("ssh".to_string());
    }
    if let Some(code) = exit {
        badges.push(format!("exit {code}"));
    }

    let btn_w = BTN + 8.0;
    let badge_w = badges
        .iter()
        .map(|b| 26.0 + b.len() as f32 * 6.5)
        .sum::<f32>();
    let title_rect = Rect::from_min_max(
        rect.min + Vec2::new(8.0, 2.0),
        rect.right_top() + Vec2::new(-(btn_w + badge_w + 8.0), rect.height() - 2.0),
    );

    let editing = st
        .win()
        .is_some_and(|w| w.ui.pane_edit.as_ref().is_some_and(|(p, _)| *p == pane));
    if editing {
        // The editor buffer lives in the window ui while validation needs
        // the pane map: field-split AppState for disjoint borrows.
        let wi = st.active_idx();
        let AppState { panes, windows, .. } = st;
        let Some(WindowState { ui: wui, .. }) = windows.get_mut(wi) else {
            return;
        };
        let buf = match wui.pane_edit.as_mut() {
            Some((_, b)) => b,
            None => return,
        };
        let resp = ui.put(
            title_rect,
            TextEdit::singleline(buf).desired_width(title_rect.width()),
        );
        resp.request_focus();
        // egui TextEdit keeps focus on Escape, so react to the keys
        // directly instead of waiting for lost_focus.
        let confirm = ui.input(|i| i.key_pressed(Key::Enter));
        // A rejected editor keeps focus (and with it the whole keyboard):
        // any click outside the field closes it, or the app is stuck.
        let outside = ui.input(|i| {
            i.pointer.any_click()
                && i.pointer
                    .interact_pos()
                    .is_none_or(|p| !title_rect.contains(p))
        });
        let cancel = ui.input(|i| i.key_pressed(Key::Escape)) || outside;
        if confirm || cancel {
            // Manual titles are the control-socket addressing key: reject
            // duplicates; pure digits would parse as a pane Id in ctl's
            // untagged PaneSelector. Keep the editor open and say why.
            let reason = if confirm {
                reject_reason(panes, buf.trim(), pane)
            } else {
                None
            };
            if reason.is_none() {
                if confirm {
                    let value = buf.trim().to_string();
                    if let Some(m) = panes.get_mut(&pane) {
                        m.manual_title = if value.is_empty() { None } else { Some(value) };
                    }
                    *dirty = true;
                }
                wui.pane_edit = None;
            }
            wui.pane_edit_note = reason;
        }
        if let Some(note) = wui.pane_edit_note {
            ui.painter().text(
                rect.left_bottom() + Vec2::new(8.0, 2.0),
                egui::Align2::LEFT_TOP,
                note,
                egui::FontId::proportional(11.0),
                to_c32(pal.bright[1]),
            );
        }
    } else {
        // Focused pane's title is full foreground; the rest dim to the
        // chrome text step.
        let fg = if focused {
            to_c32(pal.foreground)
        } else {
            to_c32(colors::title_text(pal))
        };
        ui.painter().text(
            title_rect.center(),
            egui::Align2::CENTER_CENTER,
            title.as_str(),
            egui::FontId::monospace(12.0),
            fg,
        );
        let mut x = title_rect.right() + 6.0;
        for b in badges {
            ui.painter().text(
                egui::pos2(x, title_rect.center().y),
                egui::Align2::LEFT_CENTER,
                b.as_str(),
                egui::FontId::monospace(11.0),
                to_c32(pal.bright[3]),
            );
            x += 26.0 + b.len() as f32 * 6.5;
        }
        let hit = ui.interact(
            title_rect,
            Id::new("pane_title").with(pane),
            egui::Sense::click(),
        );
        if hit.double_clicked() {
            if let Some(w) = st.win_mut() {
                w.ui.pane_edit = Some((pane, title));
                w.ui.pane_edit_note = None;
            }
        }
    }

    // X close button (appearance lives in the global Settings panel).
    let r = rect.right_top() + Vec2::new(-BTN - 4.0, (rect.height() - BTN) / 2.0);
    let close = ui.put(
        Rect::from_min_size(r, Vec2::splat(BTN)),
        Button::new("X").small(),
    );

    if close.clicked() {
        actions::do_close_pane(st, sess, uist, tab, pane, dirty);
    }
}

/// Header primary-drag intent: rearrange panes when Ctrl is held or the
/// tab holds more than one pane to rearrange against; a lone pane keeps
/// the OS-window drag (there is nothing to drop against).
pub fn header_starts_pane_move(ctrl: bool, tab_panes: usize) -> bool {
    ctrl || tab_panes >= 2
}

/// Pane context menu (attached to the pane body by the renderer).
pub fn menu(
    ui: &mut Ui,
    pane: PaneId,
    tab: usize,
    st: &mut AppState,
    sess: &mut SessionMap,
    uist: &mut UiState,
    dirty: &mut bool,
) {
    let title = st
        .panes
        .get(&pane)
        .and_then(|m| m.manual_title.clone())
        .unwrap_or_default();
    if ui.button("Rename pane").clicked() {
        if let Some(w) = st.win_mut() {
            w.ui.pane_edit = Some((pane, title));
            w.ui.pane_edit_note = None;
        }
    }
    if ui.button("Split horizontally").clicked() {
        actions::apply_pane_action(
            st,
            sess,
            uist,
            tab,
            pane,
            PaneAction::SplitHorizontal,
            dirty,
        );
    }
    if ui.button("Split vertically").clicked() {
        actions::apply_pane_action(st, sess, uist, tab, pane, PaneAction::SplitVertical, dirty);
    }
    if ui.button("Zoom pane").clicked() {
        if let Some(w) = st.win_mut() {
            w.ui.zoom = !w.ui.zoom;
        }
    }
    // The tab bar's settings cell is the primary entry point; the pane
    // menu keeps a redundant one for quick access. The flag is
    // per-window: the panel opens in THIS window.
    if ui.button("Settings").clicked() {
        if let Some(w) = st.win_mut() {
            w.ui.inspector = !w.ui.inspector;
        }
    }
    if ui.button("Respawn pane").clicked() {
        actions::apply_pane_action(st, sess, uist, tab, pane, PaneAction::Respawn, dirty);
    }
    ui.separator();
    if ui.button("Close pane").clicked() {
        actions::apply_pane_action(st, sess, uist, tab, pane, PaneAction::Close, dirty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{fresh_state, split_tree_pane};
    use layout_tree::Axis;

    #[test]
    fn header_drag_rearranges_with_a_sibling_or_ctrl() {
        assert!(header_starts_pane_move(false, 2), "sibling pane -> move");
        assert!(header_starts_pane_move(true, 1), "ctrl forces the move");
        assert!(
            !header_starts_pane_move(false, 1),
            "lone pane keeps the window drag"
        );
    }

    #[test]
    fn reject_reason_blocks_digits_and_duplicates_only() {
        let mut st = fresh_state();
        st.panes.get_mut(&1).unwrap().manual_title = Some("agent".into());
        assert_eq!(
            reject_reason(&st.panes, "", 1),
            None,
            "empty clears the title"
        );
        assert_eq!(
            reject_reason(&st.panes, "agent", 1),
            None,
            "own title is fine"
        );
        assert!(
            reject_reason(&st.panes, "7", 1).is_some(),
            "digits-only -> id"
        );
        assert_eq!(
            reject_reason(&st.panes, "42x", 1),
            None,
            "digits plus text is fine"
        );
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        // from the new pane's side, pane 1's claim is a conflict
        assert_eq!(
            reject_reason(&st.panes, "agent", 2),
            Some("another pane already uses this name")
        );
    }
}
