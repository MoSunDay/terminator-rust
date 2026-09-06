//! Per-frame rendering: pane layout, headers, terminal grids, chrome.

use std::time::Duration;

use egui::{Color32, Rect, Stroke, StrokeKind, Ui};
use layout_tree::{content_rect, layout_tab, PaneId};
use remote::PaneKind;

use crate::actions;
use crate::input::{mouse, pointer};
use crate::render::{colors, grid};
use crate::session_map;
use crate::state::{AppState, Data, UiState, DIVIDER_W, PANE_HEADER_H};
use crate::ui::pane_header;

/// Blink duty cycle: visible 1.2s of every 2s.
fn cursor_on(ctx: &egui::Context) -> bool {
    let t = ctx.input(|i| i.time);
    (t * 2.0) % 2.0 < 1.2
}

/// Pane rects (screen-space layout_tree rects) for the active tab.
fn pane_rects(st: &AppState, uist: &UiState, area: Rect) -> Vec<(PaneId, layout_tree::Rect)> {
    let Some(tab) = st.tree.tabs.get(st.tree.active_tab) else {
        return Vec::new();
    };
    let lt_area = grid::lt_rect(area);
    if uist.zoom {
        return vec![(tab.focused, lt_area)];
    }
    layout_tab(tab, lt_area, PANE_HEADER_H, DIVIDER_W)
}

/// Slim scrollbar on the right edge of a pane's content, shown only while
/// the user reviews scrollback (viewport detached from the live area).
/// `geo` is `(offset, total, len)` in rows from `viewport::geometry`.
fn draw_viewport_bar(painter: &egui::Painter, content: Rect, geo: (u64, u64, u64), thumb: Color32) {
    let (offset, total, len) = geo;
    if total == 0 || total <= len {
        return; // no history to review
    }
    let h = content.height();
    let track = Rect::from_min_max(
        egui::pos2(content.right() - 3.0, content.top()),
        egui::pos2(content.right() - 1.0, content.bottom()),
    );
    painter.rect_filled(track, 0.0, Color32::from_gray(60));
    let ratio = |v: u64| v as f32 / total as f32;
    let thumb_h = (h * ratio(len)).clamp(12.0, h);
    let top = (h * ratio(offset)).min(h - thumb_h);
    let bar = Rect::from_min_max(
        egui::pos2(track.left(), content.top() + top),
        egui::pos2(track.right(), content.top() + top + thumb_h),
    );
    painter.rect_filled(bar, 0.0, thumb);
}

/// Render the whole terminal area into the central panel ui.
pub fn screen(ui: &mut Ui, d: &mut Data) {
    let ctx = ui.ctx().clone();
    let pal = colors::palette_of(&d.st.theme_name);
    let cell = grid::measure_cells(&ctx, d.ui.font_size);

    // Lifecycle bookkeeping.
    session_map::pump_all(&mut d.sess);
    actions::auto_degrade(&mut d.st, &mut d.sess, &mut d.dirty);
    if d.st.tree.tabs.is_empty() {
        actions::do_new_tab(&mut d.st, &mut d.sess, PaneKind::Local, &mut d.dirty);
    }
    actions::ensure_sessions(&d.st, &mut d.sess);

    let area = ui.available_rect_before_wrap();
    let Data {
        st,
        sess,
        ui: uist,
        dirty,
        ..
    } = d;
    let dragging = mouse::divider_interaction(ui, st, area, &mut uist.drag, dirty);
    if st.tree.active_tab >= st.tree.tabs.len() {
        st.tree.active_tab = st.tree.tabs.len().saturating_sub(1);
    }
    let tab = st.tree.active_tab;
    let focused = st.tree.tabs.get(tab).map(|t| t.focused);
    let blink = cursor_on(&ctx);
    let painter = ui.painter().clone();
    let rects = pane_rects(st, uist, area);

    // Raw pointer routing (reporting / selection / wheel) over pane content
    // rects; suppressed while a divider drag owns the pointer.
    if !dragging {
        let content_rects: Vec<(PaneId, Rect)> = rects
            .iter()
            .map(|(p, lt)| (*p, grid::egui_rect(content_rect(*lt, PANE_HEADER_H))))
            .collect();
        pointer::handle(&ctx, &content_rects, st, sess, uist, cell.h, dirty);
    }

    for (pane, lt) in rects {
        let full = grid::egui_rect(lt);
        if full.width() < 4.0 || full.height() < 4.0 {
            continue;
        }
        let header = Rect::from_min_size(full.min, egui::vec2(full.width(), PANE_HEADER_H));
        let content = grid::egui_rect(content_rect(lt, PANE_HEADER_H));

        pane_header::show(ui, header, pane, tab, st, sess, uist, &pal, dirty);

        let fallback = crate::state::new_pane_meta(PaneKind::Local);
        let meta = st.panes.get(&pane).unwrap_or(&fallback);
        match sess.map.get_mut(&pane) {
            Some(s) if s.exit.is_none() => {
                let fr = session_map::sync_frame(s, content.width(), content.height(), cell);
                grid::draw_frame(
                    &painter,
                    content,
                    &grid::DrawArgs {
                        fr: &fr,
                        pal: &pal,
                        meta,
                        cell,
                        font_size: uist.font_size,
                        cursor_on: blink && Some(pane) == focused,
                    },
                );
                // Scrollback review indicator over the grid, right edge.
                if Some(pane) == focused && !vt_pane::viewport::pinned(s) {
                    if let Some(geo) = vt_pane::viewport::geometry(s) {
                        draw_viewport_bar(
                            &painter,
                            content,
                            geo,
                            colors::to_c32(pal.block_highlight),
                        );
                    }
                }
            }
            _ => {
                grid::draw_dead(
                    &painter,
                    content,
                    colors::effective_bg(&pal, meta),
                    colors::to_c32(pal.foreground),
                    "process exited - respawn from the context menu",
                );
            }
        }

        // Mouse-grabbing apps own right-clicks; only local panes get the
        // context menu.
        let tracking = sess
            .map
            .get(&pane)
            .map(vt_pane::mouse::is_mouse_tracking)
            .unwrap_or(false);
        if !dragging {
            let resp = mouse::pane_interact(ui, content, pane, st, dirty);
            if !tracking {
                resp.context_menu(|menu| {
                    pane_header::menu(menu, pane, tab, st, sess, uist, dirty);
                });
            }
        }

        if Some(pane) == focused && !uist.zoom {
            painter.rect_stroke(
                full,
                2.0,
                Stroke::new(1.5, colors::to_c32(pal.block_highlight)),
                StrokeKind::Inside,
            );
        }
    }

    // Divider strips over the background (non-zoom only).
    if !uist.zoom {
        if let Some(tabref) = st.tree.tabs.get(tab) {
            for h in mouse::dividers(tabref, grid::lt_rect(area), DIVIDER_W) {
                painter.rect_filled(h.strip, 0.0, Color32::from_gray(60));
            }
        }
    }

    // Terminals stream output continuously.
    ctx.request_repaint_after(Duration::from_millis(50));
}
