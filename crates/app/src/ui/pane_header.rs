//! Per-pane header strip: title, rename, badges, color/transparency popups,
//! close button and the pane context menu.

use egui::{Button, Color32, Id, Key, Popup, Rect, TextEdit, Ui, Vec2};
use layout_tree::PaneId;
use theme::{Palette, Rgb};

use crate::actions;
use crate::render::colors::{self, to_c32};
use crate::session_map::SessionMap;
use crate::state::{effective_title, AppState, PaneAction, UiState};

const BTN: f32 = 16.0;

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

    let strip = if focused {
        to_c32(pal.block_highlight).gamma_multiply(0.55)
    } else {
        Color32::from_gray(38)
    };
    ui.painter().rect_filled(rect, 2.0, strip);

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
            if confirm {
                if let Some(m) = st.panes.get_mut(&pane) {
                    let value = buf.trim().to_string();
                    m.manual_title = if value.is_empty() { None } else { Some(value) };
                }
                *dirty = true;
            }
            uist.pane_edit = None;
        }
    } else {
        let fg = to_c32(pal.foreground);
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
    trans_popup(&trans, pane, st, uist, dirty);
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
