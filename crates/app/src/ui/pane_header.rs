//! Per-pane header strip: title, rename, badges, color/transparency popups,
//! close button and the pane context menu.

use egui::{Button, Id, Key, Popup, Rect, TextEdit, Ui, Vec2};
use layout_tree::PaneId;
use theme::{Palette, Rgb};

use crate::actions;
use crate::render::colors::{self, to_c32};
use crate::session_map::SessionMap;
use crate::state::{effective_title, AppState, PaneAction, UiState};

const BTN: f32 = 16.0;

/// None when the candidate manual title is acceptable: names must be
/// unique (control-socket addressing) and not digits-only (reserved for
/// pane ids in ctl's untagged PaneSelector).
fn reject_reason(st: &AppState, value: &str, pane: PaneId) -> Option<&'static str> {
    if value.is_empty() {
        return None;
    }
    if value.chars().all(|c| c.is_ascii_digit()) {
        return Some("digits-only names are reserved for pane ids");
    }
    if crate::state::manual_title_taken(st, value, pane) {
        return Some("another pane already uses this name");
    }
    None
}

fn parse_rgb(s: &str) -> Option<Rgb> {
    theme::parse_hex(s.trim()).ok()
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
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
    let focused = st.tree.tabs.get(tab).is_some_and(|t| t.focused == pane);
    let meta = st.panes.get(&pane);
    let osc = crate::session_map::osc_title(sess, pane);
    let exit = crate::session_map::exit_code(sess, pane);
    let title = match meta {
        Some(m) => effective_title(m.manual_title.as_deref(), &osc, &m.kind),
        None => "?".to_string(),
    };

    // Quiet chrome: the header is a flat chrome strip (the focused pane is
    // already framed by its accent stroke) with a hairline over the
    // content; focus reads through the title color.
    ui.painter()
        .rect_filled(rect, 2.0, to_c32(colors::chrome_bg(pal)));
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, to_c32(colors::hairline(pal))),
    );

    // Badges (right of the title area, before the buttons).
    let mut badges: Vec<String> = Vec::new();
    if meta.is_some_and(|m| m.degraded) {
        badges.push("ssh".to_string());
    }
    if let Some(code) = exit {
        badges.push(format!("exit {code}"));
    }

    let btn_w = BTN * 3.0 + 8.0;
    let badge_w = badges
        .iter()
        .map(|b| 26.0 + b.len() as f32 * 6.5)
        .sum::<f32>();
    let title_rect = Rect::from_min_max(
        rect.min + Vec2::new(8.0, 2.0),
        rect.right_top() + Vec2::new(-(btn_w + badge_w + 8.0), rect.height() - 2.0),
    );

    let editing = uist.pane_edit.as_ref().is_some_and(|(p, _)| *p == pane);
    if editing {
        let buf = match uist.pane_edit.as_mut() {
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
        let cancel = ui.input(|i| i.key_pressed(Key::Escape));
        if confirm || cancel {
            // Manual titles are the control-socket addressing key: reject
            // duplicates; pure digits would parse as a pane Id in ctl's
            // untagged PaneSelector. Keep the editor open and say why.
            let reason = if confirm {
                reject_reason(st, buf.trim(), pane)
            } else {
                None
            };
            if reason.is_none() {
                if confirm {
                    let value = buf.trim().to_string();
                    if let Some(m) = st.panes.get_mut(&pane) {
                        m.manual_title = if value.is_empty() { None } else { Some(value) };
                    }
                    *dirty = true;
                }
                uist.pane_edit = None;
            }
            uist.pane_edit_note = reason;
        }
        if let Some(note) = uist.pane_edit_note {
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
            title_rect.min,
            egui::Align2::LEFT_CENTER,
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
            uist.pane_edit = Some((pane, title));
            uist.pane_edit_note = None;
        }
    }

    // C / T / X buttons.
    let mut r = rect.right_top() + Vec2::new(-BTN - 4.0, (rect.height() - BTN) / 2.0);
    let close = ui.put(
        Rect::from_min_size(r, Vec2::splat(BTN)),
        Button::new("X").small(),
    );
    r.x -= BTN + 2.0;
    let trans = ui.put(
        Rect::from_min_size(r, Vec2::splat(BTN)),
        Button::new("T").small(),
    );
    r.x -= BTN + 2.0;
    let color = ui.put(
        Rect::from_min_size(r, Vec2::splat(BTN)),
        Button::new("C").small(),
    );

    if close.clicked() {
        actions::do_close_pane(st, sess, uist, tab, pane, dirty);
        return;
    }
    if trans.clicked() {
        uist.trans_open = if uist.trans_open == Some(pane) {
            None
        } else {
            Some(pane)
        };
    }
    if color.clicked() {
        uist.color_open = if uist.color_open == Some(pane) {
            None
        } else {
            Some(pane)
        };
        uist.color_buf = st
            .panes
            .get(&pane)
            .and_then(|m| m.bg_color)
            .map(hex)
            .unwrap_or_default();
    }

    color_popup(&color, pane, st, uist, pal, dirty);
    trans_popup(&trans, pane, st, uist, pal, dirty);
}

fn color_popup(
    anchor: &egui::Response,
    pane: PaneId,
    st: &mut AppState,
    uist: &mut UiState,
    pal: &Palette,
    dirty: &mut bool,
) {
    let mut open = uist.color_open == Some(pane);
    Popup::from_response(anchor)
        .id(Id::new("pane_color").with(pane))
        .open_bool(&mut open)
        .show(|p| {
            p.set_min_width(190.0);
            p.label("Background override");
            p.horizontal_wrapped(|p| {
                for sw in colors::swatches(pal) {
                    let c = to_c32(sw);
                    if p.add(Button::new("  ").fill(c))
                        .on_hover_text(hex(sw))
                        .clicked()
                    {
                        if let Some(m) = st.panes.get_mut(&pane) {
                            m.bg_color = Some(sw);
                        }
                        uist.color_buf = hex(sw);
                        *dirty = true;
                    }
                }
            });
            p.horizontal(|p| {
                p.add(
                    TextEdit::singleline(&mut uist.color_buf)
                        .desired_width(80.0)
                        .hint_text("#rrggbb"),
                );
                if parse_rgb(&uist.color_buf).is_some() && p.button("apply").clicked() {
                    if let Some(rgb) = parse_rgb(&uist.color_buf) {
                        if let Some(m) = st.panes.get_mut(&pane) {
                            m.bg_color = Some(rgb);
                        }
                        *dirty = true;
                    }
                }
                if p.button("clear").clicked() {
                    if let Some(m) = st.panes.get_mut(&pane) {
                        m.bg_color = None;
                    }
                    *dirty = true;
                }
            });
        });
    if !open && uist.color_open == Some(pane) {
        uist.color_open = None;
    }
}

fn trans_popup(
    anchor: &egui::Response,
    pane: PaneId,
    st: &mut AppState,
    uist: &mut UiState,
    pal: &Palette,
    dirty: &mut bool,
) {
    let mut open = uist.trans_open == Some(pane);
    Popup::from_response(anchor)
        .id(Id::new("pane_trans").with(pane))
        .open_bool(&mut open)
        .show(|p| {
            p.set_min_width(180.0);
            let mut value = st.panes.get(&pane).map(|m| m.transparency).unwrap_or(0.0);
            if p.add(egui::Slider::new(&mut value, 0.0..=1.0).text("pane bg"))
                .changed()
            {
                if let Some(m) = st.panes.get_mut(&pane) {
                    m.transparency = value;
                }
                *dirty = true;
            }
            p.label("0 = solid pane bg, 1 = theme bg");
            if st.panes.get(&pane).is_some_and(|m| m.bg_color.is_none()) {
                p.colored_label(
                    to_c32(pal.bright[3]),
                    "no pane bg set: transparency blends the pane bg with the theme bg — set a pane bg color first (C), otherwise the slider has no visual effect",
                );
            }
        });
    if !open && uist.trans_open == Some(pane) {
        uist.trans_open = None;
    }
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
        uist.pane_edit = Some((pane, title));
        uist.pane_edit_note = None;
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
        uist.zoom = !uist.zoom;
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
    fn reject_reason_blocks_digits_and_duplicates_only() {
        let mut st = fresh_state();
        st.panes.get_mut(&1).unwrap().manual_title = Some("agent".into());
        assert_eq!(reject_reason(&st, "", 1), None, "empty clears the title");
        assert_eq!(reject_reason(&st, "agent", 1), None, "own title is fine");
        assert!(reject_reason(&st, "7", 1).is_some(), "digits-only -> id");
        assert_eq!(
            reject_reason(&st, "42x", 1),
            None,
            "digits plus text is fine"
        );
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        // from the new pane's side, pane 1's claim is a conflict
        assert_eq!(
            reject_reason(&st, "agent", 2),
            Some("another pane already uses this name")
        );
    }
}
