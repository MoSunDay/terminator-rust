//! Chrome top bar: custom-drawn tab chip row + anchored edge cells.

use egui::{pos2, vec2, Align2, CornerRadius, FontId, Id, Key, Rect, Sense, TextEdit, Ui};
use layout_tree::Tab;
use theme::Palette;

use crate::actions;
use crate::render::colors::{self, to_c32};
use crate::render::tokens;
use crate::state::{self, AppState, Data, WindowState};
use crate::ui::tabs_widgets;

const CHIP_H: f32 = 28.0;
const CHIP_PAD_X: f32 = 10.0;
const CHIP_MIN_W: f32 = 44.0;
const CHIP_GAP: f32 = 5.0;
const CLOSE_W: f32 = 14.0;
const ACCENT_H: f32 = 2.0; // active-chip underline height
/// Chip silhouette: full pill - the bar bg matches the chip field, so the
/// rounding reads as a soft silhouette instead of a tab-flap.
const CHIP_RADIUS: CornerRadius = CornerRadius {
    nw: tokens::R_MD,
    ne: tokens::R_MD,
    sw: tokens::R_MD,
    se: tokens::R_MD,
};

/// Render the single-row chrome bar: tab chips + trailing buttons, with
/// zoom + inspector cells anchored at the right edge. Mutates state via
/// user interactions only.
pub fn bar(ui: &mut Ui, d: &mut Data) {
    let pal = colors::palette_of(&d.st.theme_name);
    tab_row(ui, d, &pal);
}

/// Chip label: pane count suffix once the tab holds more than one pane.
fn chip_label(tab: &Tab) -> String {
    let panes = layout_tree::pane_count(&tab.root);
    if panes > 1 {
        format!("{} [{}]", tab.title, panes)
    } else {
        tab.title.clone()
    }
}

fn tab_row(ui: &mut Ui, d: &mut Data, pal: &Palette) {
    let row = ui.max_rect();
    // Borderless window: dragging the bare chrome (not a chip or button)
    // moves the window. Registered first so widgets added later (and thus
    // on top) keep their clicks; drags fall through to this background.
    // A chip reorder drag owns the pointer: no window move then.
    let chip_reorder = d.st.win().is_some_and(|w| w.ui.tab_drag.is_some());
    let drag = ui.interact(row, Id::new("chrome_drag"), Sense::drag());
    if !chip_reorder && drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    ui.add_space(4.0);
    // Chip slot centers (center_x, tab index) plus the first chip's top:
    // consumed by the ghost/reorder pass after the row is allocated.
    let mut centers: Vec<(f32, usize)> = Vec::new();
    let mut chip_top: Option<f32> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = CHIP_GAP;
        let Data {
            st,
            sess,
            ui: uist,
            dirty,
            ..
        } = d;
        let mut close_tab: Option<usize> = None;
        let count = st.win().map(|w| w.tree.tabs.len()).unwrap_or(0);
        let win_id = st.win().map(|w| w.id).unwrap_or(0);
        let chrome_base = to_c32(colors::chrome_bg(pal));
        let chrome_hover = to_c32(colors::chrome_hover(pal));
        for i in 0..count {
            let Some((anchor, title, label)) = st
                .win()
                .and_then(|w| w.tree.tabs.get(i))
                .map(|t| (state::tab_anchor(t), t.title.clone(), chip_label(t)))
            else {
                continue;
            };
            let editing = st
                .win()
                .is_some_and(|w| w.ui.tab_edit.as_ref().is_some_and(|(a, _)| *a == anchor));
            let selected = st.win().is_some_and(|w| w.tree.active_tab == i);
            if editing {
                // Rename buffer lives in the window ui, the title it edits
                // in the window tree: split WindowState for both.
                let wi = st.active_idx();
                let AppState { windows, .. } = st;
                let Some(WindowState { tree, ui: wui, .. }) = windows.get_mut(wi) else {
                    continue;
                };
                let Some((anchor, buf)) = wui.tab_edit.as_mut() else {
                    continue;
                };
                let resp = ui.add(
                    TextEdit::singleline(buf)
                        .desired_width(110.0)
                        .hint_text("tab title"),
                );
                resp.request_focus();
                let confirm = ui.input(|inp| inp.key_pressed(Key::Enter));
                // Same trap as the pane rename editor: a click anywhere
                // outside the field must close it.
                let outside = ui.input(|inp| {
                    inp.pointer.any_click()
                        && inp
                            .pointer
                            .interact_pos()
                            .is_none_or(|p| !resp.rect.contains(p))
                });
                let cancel = ui.input(|inp| inp.key_pressed(Key::Escape)) || outside;
                // egui TextEdit keeps focus on Escape, so react to the keys
                // directly instead of waiting for lost_focus.
                if confirm || cancel {
                    if confirm {
                        if let Some(t) = tree
                            .tabs
                            .iter_mut()
                            .find(|t| state::tab_anchor(t) == *anchor)
                        {
                            t.title = buf.clone();
                        }
                        *dirty = true;
                    }
                    wui.tab_edit = None;
                }
                continue;
            }

            let tab_drag = st.win().and_then(|w| w.ui.tab_drag);
            let painter = ui.painter().clone();
            let galley = painter.layout_no_wrap(
                label,
                FontId::proportional(12.0),
                egui::Color32::PLACEHOLDER,
            );
            let w = (CHIP_PAD_X * 2.0 + galley.size().x + CLOSE_W).max(CHIP_MIN_W);
            if tab_drag.is_some_and(|td| td.anchor == anchor) {
                // The dragged chip's slot stays an empty gap: the other
                // chips shift around it live while the ghost follows the
                // pointer (painted after the row, above the edge cells).
                let (rect, _) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::hover());
                centers.push((rect.center().x, i));
                chip_top = chip_top.or(Some(rect.top()));
                continue;
            }
            // click_and_drag: egui disambiguates by pointer movement, so
            // click-to-switch / double-click rename / middle-click close
            // all keep working next to the reorder drag.
            let (rect, resp) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::click_and_drag());
            centers.push((rect.center().x, i));
            chip_top = chip_top.or(Some(rect.top()));
            // Chrome behavior: a drag also selects the tab. The latch is
            // keyed by the tab anchor, immune to the index shifts the
            // reorder itself causes.
            if tab_drag.is_none() && resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(px) = ui.input(|i| i.pointer.interact_pos()).map(|p| p.x) {
                    if let Some(w) = st.win_mut() {
                        w.ui.tab_drag = Some(state::TabDrag {
                            anchor,
                            grab_dx: px - rect.left(),
                            w: rect.width(),
                        });
                        w.tree.active_tab = i;
                        w.ui.zoom = false;
                    }
                    *dirty = true;
                }
            }
            if selected {
                painter.rect_filled(rect, CHIP_RADIUS, to_c32(colors::tab_active(pal)));
                // Rounded-cap accent underline flush at the chip bottom:
                // this tab owns the content below. Inset clear of the pill
                // corners so the caps stay on the straight edge.
                let underline = Rect::from_min_max(
                    pos2(
                        rect.left() + f32::from(tokens::R_MD) + 2.0,
                        rect.bottom() - ACCENT_H,
                    ),
                    pos2(rect.right() - f32::from(tokens::R_MD) - 2.0, rect.bottom()),
                );
                painter.rect_filled(underline, 1.0, to_c32(pal.block_highlight));
            } else {
                let t = tokens::hover_t(
                    ui.ctx(),
                    Id::new("chip_fade").with(win_id).with(i),
                    resp.hovered(),
                );
                if t > 0.0 {
                    painter.rect_filled(
                        rect,
                        CHIP_RADIUS,
                        tokens::lerp_color(chrome_base, chrome_hover, t),
                    );
                }
            }
            let text_col = if selected {
                to_c32(pal.foreground)
            } else {
                tabs_widgets::dim_text(pal)
            };
            let galley_rect = Align2::LEFT_CENTER.align_size_within_rect(
                galley.size(),
                Rect::from_min_max(
                    pos2(rect.left() + CHIP_PAD_X, rect.top()),
                    pos2(rect.right() - CLOSE_W, rect.bottom()),
                ),
            );
            painter.galley(galley_rect.min, galley, text_col);

            // Close affordance: right-hand strip, revealed on hover/active.
            let close_rect = Rect::from_center_size(
                pos2(rect.right() - CLOSE_W * 0.5, rect.center().y),
                vec2(12.0, 12.0),
            );
            let close_resp = ui.interact(close_rect, Id::new("tab_close").with(i), Sense::click());
            if selected || resp.hovered() || close_resp.hovered() {
                let xcol = if selected {
                    to_c32(pal.foreground)
                } else {
                    tabs_widgets::dim_text(pal)
                };
                tabs_widgets::close_glyph(&painter, close_rect, close_resp.hovered(), pal, xcol);
            }
            if close_resp.clicked() {
                close_tab = Some(i);
            } else if resp.clicked() && !selected {
                if let Some(w) = st.win_mut() {
                    w.tree.active_tab = i;
                    w.ui.zoom = false;
                }
            }
            close_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.double_clicked() {
                if let Some(w) = st.win_mut() {
                    w.ui.tab_edit = Some((anchor, title));
                }
            }
            if resp.middle_clicked() {
                close_tab = Some(i);
            }
        }
        if let Some(i) = close_tab {
            actions::do_close_tab(st, sess, uist, i, dirty);
        }

        ui.add_space(8.0);
        tabs_widgets::trailing_buttons(ui, st, sess, pal, dirty);
        tabs_widgets::edge_cells(ui, row.right(), st);
    });
    // In-flight chip drag: live reorder + ghost chip. Runs after the row
    // closure so the ghost paints above the trailing buttons/edge cells;
    // the anchor pane closing mid-drag (impossible from the bar itself,
    // reachable via ctl) drops the drag.
    if let Some(td) = d.st.win().and_then(|w| w.ui.tab_drag) {
        let cur = d.st.win().and_then(|w| {
            w.tree
                .tabs
                .iter()
                .position(|t| state::tab_anchor(t) == td.anchor)
        });
        let pos = ui.input(|i| i.pointer.interact_pos());
        let held = pos.is_some() && ui.input(|i| i.pointer.primary_down());
        match cur {
            None => {
                if let Some(w) = d.st.win_mut() {
                    w.ui.tab_drag = None;
                }
            }
            Some(cur) => {
                if let Some(pos) = pos {
                    let ghost_left = (pos.x - td.grab_dx)
                        .clamp(row.left(), (row.right() - td.w).max(row.left()));
                    // Slots to the left of the ghost center, excluding the
                    // dragged one, give the insertion index.
                    let target = centers
                        .iter()
                        .filter(|(cx, i)| *i != cur && *cx < ghost_left + td.w * 0.5)
                        .count();
                    if target != cur {
                        if let Some(w) = d.st.win_mut() {
                            if layout_tree::move_tab(&mut w.tree, cur, target) {
                                d.dirty = true;
                            }
                        }
                    }
                    if held {
                        let label =
                            d.st.win()
                                .and_then(|w| {
                                    w.tree
                                        .tabs
                                        .iter()
                                        .find(|t| state::tab_anchor(t) == td.anchor)
                                })
                                .map(chip_label);
                        if let (Some(label), Some(top)) = (label, chip_top) {
                            ghost_chip(ui, pal, &label, ghost_left, top, td.w);
                        }
                    }
                }
                if !held {
                    if let Some(w) = d.st.win_mut() {
                        w.ui.tab_drag = None;
                    }
                }
            }
        }
    }
    ui.add_space(4.0);
    // Hairline under the merged chrome row (was under the removed title
    // row): a quiet seam above the pane area.
    ui.painter().hline(
        row.x_range(),
        ui.min_rect().bottom() - 0.5,
        egui::Stroke::new(1.0, to_c32(colors::hairline(pal))),
    );
}

/// Ghost chip for the in-flight tab drag: selected-chip look pinned under
/// the pointer. The label centers (the real chip left-aligns to leave
/// room for its close button, which the ghost does not carry).
fn ghost_chip(ui: &mut Ui, pal: &Palette, label: &str, left: f32, top: f32, w: f32) {
    let painter = ui.painter().clone();
    let rect = Rect::from_min_size(pos2(left, top), vec2(w, CHIP_H));
    painter.rect_filled(rect, CHIP_RADIUS, to_c32(colors::tab_active(pal)));
    // Rounded-cap accent underline, inset like the real chip.
    let underline = Rect::from_min_max(
        pos2(
            rect.left() + f32::from(tokens::R_MD) + 2.0,
            rect.bottom() - ACCENT_H,
        ),
        pos2(rect.right() - f32::from(tokens::R_MD) - 2.0, rect.bottom()),
    );
    painter.rect_filled(underline, 1.0, to_c32(pal.block_highlight));
    let galley = painter.layout_no_wrap(
        label.to_owned(),
        FontId::proportional(12.0),
        egui::Color32::PLACEHOLDER,
    );
    let text_rect = Align2::CENTER_CENTER.align_size_within_rect(galley.size(), rect);
    painter.galley(text_rect.min, galley, to_c32(pal.foreground));
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout_tree::{new_tree, split_pane, Axis};

    #[test]
    fn chip_label_counts_panes() {
        let mut tree = new_tree("work");
        assert_eq!(chip_label(&tree.tabs[0]), "work");
        let focused = tree.tabs[0].focused;
        assert_eq!(
            split_pane(&mut tree, 0, focused, Axis::Vertical),
            Some(2),
            "split creates the second pane"
        );
        assert_eq!(chip_label(&tree.tabs[0]), "work [2]");
    }
}
