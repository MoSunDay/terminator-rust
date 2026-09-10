//! Chrome top bar: centered title row + custom-drawn tab chip row.

use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontId, Id, Key, Pos2, Rect, Sense, Stroke,
    StrokeKind, TextEdit, Ui,
};
use layout_tree::{Axis, Tab};
use remote::PaneKind;
use theme::Palette;

use crate::actions;
use crate::render::colors::{self, to_c32};
use crate::session_map::SessionMap;
use crate::state::{self, AppState, Data};

const CHIP_H: f32 = 24.0;
const CHIP_PAD_X: f32 = 10.0;
const CHIP_MIN_W: f32 = 44.0;
const CHIP_GAP: f32 = 5.0;
const CLOSE_W: f32 = 14.0;
const ACCENT_H: f32 = 2.0; // active-chip underline height
const ICON: f32 = 16.0; // icon button cell
/// Chip silhouette: rounded top corners, square where it meets the content.
const CHIP_RADIUS: CornerRadius = CornerRadius {
    nw: 5,
    ne: 5,
    sw: 0,
    se: 0,
};

/// Render the single-row chrome bar: tab chips + trailing buttons, with
/// zoom + inspector cells anchored at the right edge. Mutates state via
/// user interactions only.
pub fn bar(ui: &mut Ui, d: &mut Data) {
    let pal = colors::palette_of(&d.st.theme_name);
    tab_row(ui, d, &pal);
}

/// Chrome text color helper (dimmed).
fn dim_text(pal: &Palette) -> Color32 {
    to_c32(colors::title_text(pal))
}

/// Hover fill for a chrome icon cell.
fn hover_fill(ui: &Ui, rect: Rect, hovered: bool, pal: &Palette) {
    if hovered {
        ui.painter()
            .rect_filled(rect, 4.0, to_c32(colors::chrome_hover(pal)));
    }
}

/// Corner brackets marking the zoomed (single-pane) view.
fn corner_brackets(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = 4.5; // half cell
    let l = 3.0; // arm length
    let st = Stroke::new(1.5, color);
    let corner = |px: f32, py: f32, dx: f32, dy: f32| {
        p.line_segment([pos2(px, py + dy * l), pos2(px, py)], st);
        p.line_segment([pos2(px, py), pos2(px + dx * l, py)], st);
    };
    corner(c.x - s, c.y - s, 1.0, 1.0); // top-left
    corner(c.x + s, c.y - s, -1.0, 1.0); // top-right
    corner(c.x - s, c.y + s, 1.0, -1.0); // bottom-left
    corner(c.x + s, c.y + s, -1.0, -1.0);
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
    let drag = ui.interact(row, Id::new("chrome_drag"), Sense::drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    ui.add_space(3.0);
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
        let count = st.tree.tabs.len();
        for i in 0..count {
            let Some((anchor, title, label)) = st
                .tree
                .tabs
                .get(i)
                .map(|t| (state::tab_anchor(t), t.title.clone(), chip_label(t)))
            else {
                continue;
            };
            let editing = uist.tab_edit.as_ref().is_some_and(|(a, _)| *a == anchor);
            let selected = i == st.tree.active_tab;
            if editing {
                let Some((anchor, buf)) = uist.tab_edit.as_mut() else {
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
                        if let Some(t) = st
                            .tree
                            .tabs
                            .iter_mut()
                            .find(|t| state::tab_anchor(t) == *anchor)
                        {
                            t.title = buf.clone();
                        }
                        *dirty = true;
                    }
                    uist.tab_edit = None;
                }
                continue;
            }

            let painter = ui.painter().clone();
            let galley =
                painter.layout_no_wrap(label, FontId::proportional(11.5), Color32::PLACEHOLDER);
            let w = (CHIP_PAD_X * 2.0 + galley.size().x + CLOSE_W).max(CHIP_MIN_W);
            let (rect, resp) = ui.allocate_exact_size(vec2(w, CHIP_H), Sense::click());
            if selected {
                painter.rect_filled(rect, CHIP_RADIUS, to_c32(colors::tab_active(pal)));
                // Underline flush at the chip bottom: this tab owns the
                // content below.
                let underline = Rect::from_min_max(
                    pos2(rect.left(), rect.bottom() - ACCENT_H),
                    pos2(rect.right(), rect.bottom()),
                );
                painter.rect_filled(underline, 0.0, to_c32(pal.block_highlight));
            } else if resp.hovered() {
                painter.rect_filled(rect, CHIP_RADIUS, to_c32(colors::chrome_hover(pal)));
            }
            let text_col = if selected {
                to_c32(pal.foreground)
            } else {
                dim_text(pal)
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
                    dim_text(pal)
                };
                let xstroke = Stroke::new(1.2, xcol);
                let c = close_rect.center();
                let a = 3.5;
                painter.line_segment([pos2(c.x - a, c.y - a), pos2(c.x + a, c.y + a)], xstroke);
                painter.line_segment([pos2(c.x - a, c.y + a), pos2(c.x + a, c.y - a)], xstroke);
            }
            if close_resp.clicked() {
                close_tab = Some(i);
            } else if resp.clicked() && !selected {
                st.tree.active_tab = i;
                uist.zoom = false;
            }
            close_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.double_clicked() {
                uist.tab_edit = Some((anchor, title));
            }
            if resp.middle_clicked() {
                close_tab = Some(i);
            }
        }
        if let Some(i) = close_tab {
            actions::do_close_tab(st, sess, uist, i, dirty);
        }

        ui.add_space(8.0);
        trailing_buttons(ui, st, sess, pal, dirty);

        // Right-edge cells anchored to the row end (old title-row place):
        // zoom left of inspector. interact() on fixed rects, layout cursor
        // untouched.
        let band = ui.min_rect();
        let iy = (band.top() + band.bottom()) / 2.0 - ICON / 2.0;
        let insp_rect = Rect::from_min_size(pos2(row.right() - 5.0 - ICON, iy), vec2(ICON, ICON));
        let zoom_rect = Rect::from_min_size(
            pos2(insp_rect.left() - 4.0 - ICON, insp_rect.top()),
            vec2(ICON, ICON),
        );
        let painter = ui.painter().clone();

        let zoom = ui.interact(zoom_rect, Id::new("chrome_zoom"), Sense::click());
        hover_fill(ui, zoom_rect, zoom.hovered(), pal);
        // Quiet accent hint when idle; full accent once zoomed.
        let zoom_col = if uist.zoom {
            to_c32(pal.block_highlight)
        } else {
            to_c32(pal.block_highlight).gamma_multiply(0.7)
        };
        corner_brackets(&painter, zoom_rect.center(), zoom_col);
        if zoom.clicked() {
            uist.zoom = !uist.zoom;
        }
        zoom.on_hover_text("Zoom focused pane (Ctrl+Shift+F)");

        let insp = ui.interact(insp_rect, Id::new("chrome_inspector"), Sense::click());
        hover_fill(ui, insp_rect, insp.hovered(), pal);
        let insp_col = if uist.inspector {
            to_c32(pal.block_highlight)
        } else {
            dim_text(pal)
        };
        painter.text(
            insp_rect.center(),
            Align2::CENTER_CENTER,
            "i",
            FontId::proportional(12.5),
            insp_col,
        );
        if insp.clicked() {
            uist.inspector = !uist.inspector;
        }
        insp.on_hover_text("Inspector (settings, hosts)");
    });
    ui.add_space(3.0);
    // Hairline under the merged chrome row (was under the removed title
    // row): a quiet seam above the pane area.
    ui.painter().hline(
        row.x_range(),
        ui.min_rect().bottom() - 0.5,
        Stroke::new(1.0, to_c32(colors::hairline(pal))),
    );
}

/// End-of-row icon buttons: new tab + explicit-axis splits.
fn trailing_buttons(
    ui: &mut Ui,
    st: &mut AppState,
    sess: &mut SessionMap,
    pal: &Palette,
    dirty: &mut bool,
) {
    let painter = ui.painter().clone();
    let icon_col = |hovered: bool| {
        to_c32(if hovered {
            pal.foreground
        } else {
            colors::title_text(pal)
        })
    };

    let (rect, resp) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    hover_fill(ui, rect, resp.hovered(), pal);
    let c = rect.center();
    let st_line = Stroke::new(1.5, icon_col(resp.hovered()));
    painter.line_segment([pos2(c.x - 4.0, c.y), pos2(c.x + 4.0, c.y)], st_line);
    painter.line_segment([pos2(c.x, c.y - 4.0), pos2(c.x, c.y + 4.0)], st_line);
    if resp.clicked() {
        actions::do_new_tab(st, sess, PaneKind::Local, dirty);
    }
    resp.on_hover_text("New tab (Ctrl+Shift+T)");

    let (rect, resp) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    hover_fill(ui, rect, resp.hovered(), pal);
    split_icon(
        &painter,
        rect.center(),
        Axis::Vertical,
        icon_col(resp.hovered()),
    );
    if resp.clicked() {
        actions::do_split(st, sess, st.tree.active_tab, None, Axis::Vertical, dirty);
    }
    resp.on_hover_text("Split left / right (Ctrl+Shift+E)");

    let (rect, resp) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::click());
    hover_fill(ui, rect, resp.hovered(), pal);
    split_icon(
        &painter,
        rect.center(),
        Axis::Horizontal,
        icon_col(resp.hovered()),
    );
    if resp.clicked() {
        actions::do_split(st, sess, st.tree.active_tab, None, Axis::Horizontal, dirty);
    }
    resp.on_hover_text("Split top / bottom (Ctrl+Shift+O)");
}

/// Two small outlined panes along the split axis.
fn split_icon(p: &egui::Painter, c: Pos2, axis: Axis, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    let (first, second) = match axis {
        // Vertical divider: children side by side (6x9 each, 2 gap).
        Axis::Vertical => (
            Rect::from_min_max(pos2(c.x - 7.0, c.y - 4.5), pos2(c.x - 1.0, c.y + 4.5)),
            Rect::from_min_max(pos2(c.x + 1.0, c.y - 4.5), pos2(c.x + 7.0, c.y + 4.5)),
        ),
        // Horizontal divider: children stacked (9x6 each).
        Axis::Horizontal => (
            Rect::from_min_max(pos2(c.x - 4.5, c.y - 7.0), pos2(c.x + 4.5, c.y - 1.0)),
            Rect::from_min_max(pos2(c.x - 4.5, c.y + 1.0), pos2(c.x + 4.5, c.y + 7.0)),
        ),
    };
    p.rect_stroke(first, 2.0, stroke, StrokeKind::Middle);
    p.rect_stroke(second, 2.0, stroke, StrokeKind::Middle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout_tree::{new_tree, split_pane};

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
