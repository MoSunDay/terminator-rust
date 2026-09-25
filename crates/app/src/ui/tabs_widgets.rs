//! Chrome-row widgetry: trailing icon buttons, the right-edge anchor
//! cells (zoom / inspector) and shared icon-painting helpers. Split from
//! tabs.rs to keep both files inside the size budget.

use egui::{
    pos2, vec2, Align2, Color32, Context, CornerRadius, FontId, Id, Painter, Pos2, Rect, Sense,
    Stroke, StrokeKind, Ui,
};
use layout_tree::Axis;
use remote::PaneKind;
use theme::Palette;

use crate::actions;
use crate::render::colors::{self, to_c32};
use crate::render::tokens;
use crate::session_map::SessionMap;
use crate::state::AppState;
use crate::ui::chrome::Metrics;

/// Chrome text color helper (dimmed).
pub fn dim_text(pal: &Palette) -> Color32 {
    to_c32(colors::title_text(pal))
}

/// Faded hover fill for a chrome cell: chrome_bg -> chrome_hover over the
/// short animation, instant when motion is disabled.
pub fn hover_fill(ui: &Ui, rect: Rect, id: Id, hovered: bool, pal: &Palette, rounding: u8) {
    let t = tokens::hover_t(ui.ctx(), id, hovered);
    if t <= 0.0 {
        return;
    }
    let col = tokens::lerp_color(
        to_c32(colors::chrome_bg(pal)),
        to_c32(colors::chrome_hover(pal)),
        t,
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(rounding), col);
}

/// Tab-chip close affordance: circular hover plate under the X glyph.
/// The arms/stroke scale off the plate rect so the X grows with the
/// chrome scale (a 12px rect is the historical 1:1 size).
pub fn close_glyph(p: &Painter, rect: Rect, hovered: bool, pal: &Palette, col: Color32) {
    if hovered {
        p.rect_filled(rect, 6.0, to_c32(colors::chrome_hover(pal)));
    }
    let k = rect.width() / 12.0;
    let xstroke = Stroke::new(1.2 * k, col);
    let c = rect.center();
    let a = 3.5 * k;
    p.line_segment([pos2(c.x - a, c.y - a), pos2(c.x + a, c.y + a)], xstroke);
    p.line_segment([pos2(c.x - a, c.y + a), pos2(c.x + a, c.y - a)], xstroke);
}

/// Corner brackets marking the zoomed (single-pane) view.
pub fn corner_brackets(p: &Painter, c: Pos2, color: Color32, s: f32) {
    let s = 4.5 * s; // half cell
    let l = 3.0 * s; // arm length
    let st = Stroke::new(1.5 * s, color);
    let corner = |px: f32, py: f32, dx: f32, dy: f32| {
        p.line_segment([pos2(px, py + dy * l), pos2(px, py)], st);
        p.line_segment([pos2(px, py), pos2(px + dx * l, py)], st);
    };
    corner(c.x - s, c.y - s, 1.0, 1.0); // top-left
    corner(c.x + s, c.y - s, -1.0, 1.0); // top-right
    corner(c.x - s, c.y + s, 1.0, -1.0); // bottom-left
    corner(c.x + s, c.y + s, -1.0, -1.0); // bottom-right
}

/// Two small outlined panes along the split axis.
fn split_icon(p: &Painter, c: Pos2, axis: Axis, color: Color32, s: f32) {
    let stroke = Stroke::new(1.4 * s, color);
    let (first, second) = match axis {
        // Vertical divider: children side by side (6x9 each, 2 gap).
        Axis::Vertical => (
            Rect::from_min_max(
                pos2(c.x - 7.0 * s, c.y - 4.5 * s),
                pos2(c.x - 1.0 * s, c.y + 4.5 * s),
            ),
            Rect::from_min_max(
                pos2(c.x + 1.0 * s, c.y - 4.5 * s),
                pos2(c.x + 7.0 * s, c.y + 4.5 * s),
            ),
        ),
        // Horizontal divider: children stacked (9x6 each).
        Axis::Horizontal => (
            Rect::from_min_max(
                pos2(c.x - 4.5 * s, c.y - 7.0 * s),
                pos2(c.x + 4.5 * s, c.y - 1.0 * s),
            ),
            Rect::from_min_max(
                pos2(c.x - 4.5 * s, c.y + 1.0 * s),
                pos2(c.x + 4.5 * s, c.y + 7.0 * s),
            ),
        ),
    };
    p.rect_stroke(first, 2.0, stroke, StrokeKind::Middle);
    p.rect_stroke(second, 2.0, stroke, StrokeKind::Middle);
}

/// End-of-row icon buttons: new tab + explicit-axis splits. Fixed-rect
/// cells pinned right of the chip strip (which scrolls under them):
/// `interact()` on computed rects, the layout cursor is untouched.
pub fn trailing_buttons(
    ui: &mut Ui,
    left: Pos2,
    st: &mut AppState,
    sess: &mut SessionMap,
    pal: &Palette,
    dirty: &mut bool,
    m: &Metrics,
) {
    let painter = ui.painter().clone();
    let icon_col = |hovered: bool| {
        to_c32(if hovered {
            pal.foreground
        } else {
            colors::title_text(pal)
        })
    };
    // Three icon cells with a gutter, anchored at `left`.
    let cell = |i: i32| {
        Rect::from_min_size(
            pos2(left.x + i as f32 * (m.icon + m.icon_gap), left.y),
            vec2(m.icon, m.icon),
        )
    };

    let rect = cell(0);
    let resp = ui.interact(rect, Id::new("chrome_newtab"), Sense::CLICK);
    hover_fill(
        ui,
        rect,
        Id::new("chrome_newtab"),
        resp.hovered(),
        pal,
        tokens::R_SM,
    );
    let c = rect.center();
    let st_line = Stroke::new(1.5 * m.s, icon_col(resp.hovered()));
    painter.line_segment(
        [pos2(c.x - 4.0 * m.s, c.y), pos2(c.x + 4.0 * m.s, c.y)],
        st_line,
    );
    painter.line_segment(
        [pos2(c.x, c.y - 4.0 * m.s), pos2(c.x, c.y + 4.0 * m.s)],
        st_line,
    );
    if resp.clicked() {
        actions::do_new_tab(st, sess, PaneKind::Local, dirty);
    }
    resp.on_hover_text("New tab (Ctrl+Shift+T)");

    let rect = cell(1);
    let resp = ui.interact(rect, Id::new("chrome_splitv"), Sense::CLICK);
    hover_fill(
        ui,
        rect,
        Id::new("chrome_splitv"),
        resp.hovered(),
        pal,
        tokens::R_SM,
    );
    split_icon(
        &painter,
        rect.center(),
        Axis::Vertical,
        icon_col(resp.hovered()),
        m.s,
    );
    if resp.clicked() {
        actions::do_split(
            st,
            sess,
            st.win().map(|w| w.tree.active_tab).unwrap_or(0),
            None,
            Axis::Vertical,
            dirty,
        );
    }
    resp.on_hover_text("Split left / right (Ctrl+Shift+E)");

    let rect = cell(2);
    let resp = ui.interact(rect, Id::new("chrome_splith"), Sense::CLICK);
    hover_fill(
        ui,
        rect,
        Id::new("chrome_splith"),
        resp.hovered(),
        pal,
        tokens::R_SM,
    );
    split_icon(
        &painter,
        rect.center(),
        Axis::Horizontal,
        icon_col(resp.hovered()),
        m.s,
    );
    if resp.clicked() {
        actions::do_split(
            st,
            sess,
            st.win().map(|w| w.tree.active_tab).unwrap_or(0),
            None,
            Axis::Horizontal,
            dirty,
        );
    }
    resp.on_hover_text("Split top / bottom (Ctrl+Shift+O)");
}

/// Whether the CURRENT viewport is in its enlarged state: maximized on
/// Linux/Windows, borderless FULLSCREEN on macOS (a "maximized" macOS
/// window only zooms inside the screen furniture - the close/max/min
/// row lives in the fullscreen space there).
pub fn window_enlarged(ctx: &Context) -> bool {
    #[cfg(target_os = "macos")]
    {
        ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
    }
    #[cfg(not(target_os = "macos"))]
    {
        ctx.input(|i| i.viewport().maximized == Some(true))
    }
}

/// Toggle the enlarged state: ViewportCommand::Fullscreen on macOS,
/// Maximized elsewhere. Sent to the CURRENT viewport (immediate pass).
pub fn send_toggle_enlarge(ctx: &Context) {
    #[cfg(target_os = "macos")]
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!window_enlarged(ctx)));
    #[cfg(not(target_os = "macos"))]
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!window_enlarged(ctx)));
}

/// Right-edge anchor cells, pinned to the row end: window controls
/// (close, maximize/restore, minimize) outermost, then the inspector
/// and zoom cells at the old title-row place. `interact()` on fixed
/// rects: the layout cursor is untouched.
pub fn edge_cells(ui: &mut Ui, row_right: f32, st: &mut AppState, m: &Metrics) {
    let pal = colors::palette_of(&st.theme_name);
    let band = ui.min_rect();
    let iy = (band.top() + band.bottom()) / 2.0 - m.icon / 2.0;
    let close_rect = Rect::from_min_size(
        pos2(row_right - 5.0 * m.s - m.icon, iy),
        vec2(m.icon, m.icon),
    );
    let max_rect = Rect::from_min_size(
        pos2(close_rect.left() - m.icon_gap - m.icon, close_rect.top()),
        vec2(m.icon, m.icon),
    );
    let min_rect = Rect::from_min_size(
        pos2(max_rect.left() - m.icon_gap - m.icon, max_rect.top()),
        vec2(m.icon, m.icon),
    );
    let insp_rect = Rect::from_min_size(
        pos2(min_rect.left() - m.icon_gap - m.icon, min_rect.top()),
        vec2(m.icon, m.icon),
    );
    let zoom_rect = Rect::from_min_size(
        pos2(insp_rect.left() - m.icon_gap - m.icon, insp_rect.top()),
        vec2(m.icon, m.icon),
    );
    let painter = ui.painter().clone();

    let zoom = ui.interact(zoom_rect, Id::new("chrome_zoom"), Sense::CLICK);
    hover_fill(
        ui,
        zoom_rect,
        Id::new("chrome_zoom_h"),
        zoom.hovered(),
        &pal,
        tokens::R_MD,
    );
    // Quiet accent hint when idle; full accent once zoomed.
    let zoom_col = if st.win().is_some_and(|w| w.ui.zoom) {
        to_c32(pal.block_highlight)
    } else {
        to_c32(pal.block_highlight).gamma_multiply(0.7)
    };
    corner_brackets(&painter, zoom_rect.center(), zoom_col, m.s);
    if zoom.clicked() {
        if let Some(w) = st.win_mut() {
            w.ui.zoom = !w.ui.zoom;
        }
    }
    zoom.on_hover_text("Zoom focused pane (Ctrl+Shift+F)");

    let insp = ui.interact(insp_rect, Id::new("chrome_inspector"), Sense::CLICK);
    hover_fill(
        ui,
        insp_rect,
        Id::new("chrome_inspector_h"),
        insp.hovered(),
        &pal,
        tokens::R_MD,
    );
    // Per-window flag: the panel opens in the window that clicked.
    let insp_col = if st.win().is_some_and(|w| w.ui.inspector) {
        to_c32(pal.block_highlight)
    } else {
        dim_text(&pal)
    };
    painter.text(
        insp_rect.center(),
        Align2::CENTER_CENTER,
        "i",
        FontId::proportional(12.5 * m.s),
        insp_col,
    );
    if insp.clicked() {
        if let Some(w) = st.win_mut() {
            w.ui.inspector = !w.ui.inspector;
        }
    }
    insp.on_hover_text("Settings (theme, font, glass, splits, hosts)");

    // Minimize: a dash ON the cell's vertical centre (a lowered dash
    // reads as '_', not '-'). The command goes to the CURRENT viewport
    // (send_viewport_cmd resolves inside the immediate pass).
    let min_col = |hovered: bool| {
        to_c32(if hovered {
            pal.foreground
        } else {
            colors::title_text(&pal)
        })
    };
    let min = ui.interact(min_rect, Id::new("chrome_min"), Sense::CLICK);
    hover_fill(
        ui,
        min_rect,
        Id::new("chrome_min_h"),
        min.hovered(),
        &pal,
        tokens::R_MD,
    );
    let c = min_rect.center();
    let line = Stroke::new(1.5 * m.s, min_col(min.hovered()));
    painter.line_segment(
        [pos2(c.x - 4.5 * m.s, c.y), pos2(c.x + 4.5 * m.s, c.y)],
        line,
    );
    if min.clicked() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
    min.on_hover_text("Minimize");

    // Maximize / restore: single square when normal, two offset squares
    // when enlarged.
    let enlarged = window_enlarged(ui.ctx());
    let max = ui.interact(max_rect, Id::new("chrome_max"), Sense::CLICK);
    hover_fill(
        ui,
        max_rect,
        Id::new("chrome_max_h"),
        max.hovered(),
        &pal,
        tokens::R_MD,
    );
    let stroke = Stroke::new(1.4 * m.s, min_col(max.hovered()));
    let c = max_rect.center();
    if enlarged {
        // Restore: back pane offset up-right, front pane down-left.
        let back = Rect::from_center_size(
            pos2(c.x + 3.0 * m.s, c.y - 3.0 * m.s),
            vec2(7.0 * m.s, 7.0 * m.s),
        );
        let front = Rect::from_center_size(
            pos2(c.x - 2.0 * m.s, c.y + 2.0 * m.s),
            vec2(9.0 * m.s, 9.0 * m.s),
        );
        painter.rect_stroke(back, 2.0, stroke, StrokeKind::Middle);
        painter.rect_stroke(front, 2.0, stroke, StrokeKind::Middle);
    } else {
        let r = Rect::from_center_size(c, vec2(9.0 * m.s, 9.0 * m.s));
        painter.rect_stroke(r, 2.0, stroke, StrokeKind::Middle);
    }
    if max.clicked() {
        send_toggle_enlarge(ui.ctx());
    }
    max.on_hover_text(if cfg!(target_os = "macos") {
        if enlarged {
            "Exit Fullscreen"
        } else {
            "Fullscreen"
        }
    } else if enlarged {
        "Restore"
    } else {
        "Maximize"
    });

    // Close (ALL windows): ONE click opens the confirmation modal
    // (ui::close_dialog); the quit itself goes out from the modal's Quit
    // button via the same ROOT close path Ctrl+Shift+Q takes. No latch
    // here - the modal owns the second step, and while it is up it is the
    // top layer, so a second press on this cell never reaches it.
    let dialog_open = st.win().is_some_and(|w| w.ui.close_dialog);
    let close = ui.interact(close_rect, Id::new("chrome_close"), Sense::CLICK);
    if dialog_open {
        ui.painter().rect_filled(
            close_rect,
            CornerRadius::same(tokens::R_MD),
            to_c32(colors::chrome_hover(&pal)),
        );
    } else {
        hover_fill(
            ui,
            close_rect,
            Id::new("chrome_close_h"),
            close.hovered(),
            &pal,
            tokens::R_MD,
        );
    }
    let close_col = if dialog_open || close.hovered() {
        to_c32(pal.normal[1])
    } else {
        dim_text(&pal)
    };
    close_glyph(&painter, close_rect.shrink(2.0), false, &pal, close_col);
    if close.clicked() {
        if let Some(w) = st.win_mut() {
            w.ui.close_dialog = true;
        }
    }
    close.on_hover_text(if dialog_open {
        "Close all windows - waiting for confirmation"
    } else {
        "Close all windows"
    });
}
