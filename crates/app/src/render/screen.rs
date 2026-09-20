//! Per-frame rendering: pane layout, headers, terminal grids, chrome.

use std::time::Duration;

use egui::{Color32, CornerRadius, Rect, Stroke, StrokeKind, Ui};
use layout_tree::{content_rect, layout_tab, PaneId};
use remote::PaneKind;

use crate::actions;
use crate::input::{mouse, pointer};
use crate::render::{colors, dropzone, grid, preedit, tokens};
use crate::session_map;
use crate::state::{AppState, Data, DIVIDER_W, PANE_HEADER_H};

/// Pane card silhouette: rounded where the header meets the chrome, square
/// at the window bottom.
const CARD_RADIUS: CornerRadius = CornerRadius {
    nw: tokens::R_MD,
    ne: tokens::R_MD,
    sw: 0,
    se: 0,
};
use crate::ui::pane_header;

/// Pane rects (screen-space layout_tree rects) for the active tab.
fn pane_rects(st: &AppState, area: Rect) -> Vec<(PaneId, layout_tree::Rect)> {
    let Some(w) = st.win() else {
        return Vec::new();
    };
    let Some(tab) = w.tree.tabs.get(w.tree.active_tab) else {
        return Vec::new();
    };
    let lt_area = grid::lt_rect(area);
    if w.ui.zoom {
        return vec![(tab.focused, lt_area)];
    }
    layout_tab(tab, lt_area, PANE_HEADER_H, DIVIDER_W)
}

/// Slim scrollbar on the right edge of a pane's content, shown only while
/// the user reviews scrollback (viewport detached from the live area).
/// `geo` is `(offset, total, len)` in rows from `viewport::geometry`.
fn draw_viewport_bar(
    painter: &egui::Painter,
    content: Rect,
    geo: (u64, u64, u64),
    thumb: Color32,
    track: Color32,
) {
    let (offset, total, len) = geo;
    if total == 0 || total <= len {
        return; // no history to review
    }
    let h = content.height();
    let track_rect = Rect::from_min_max(
        egui::pos2(content.right() - 4.0, content.top()),
        egui::pos2(content.right() - 1.0, content.bottom()),
    );
    painter.rect_filled(track_rect, 1.0, track);
    let ratio = |v: u64| v as f32 / total as f32;
    let thumb_h = (h * ratio(len)).clamp(12.0, h);
    let top = (h * ratio(offset)).min(h - thumb_h);
    let bar = Rect::from_min_max(
        egui::pos2(track_rect.left(), content.top() + top),
        egui::pos2(track_rect.right(), content.top() + top + thumb_h),
    );
    painter.rect_filled(bar, 2.0, thumb);
}

/// Render the whole terminal area into the central panel ui.
pub fn screen(ui: &mut Ui, d: &mut Data) {
    let ctx = ui.ctx().clone();
    let pal = colors::palette_of(&d.st.theme_name);
    let font_size = d.st.win().map(|w| w.ui.font_size).unwrap_or(14.0);
    let cell = grid::measure_cells(&ctx, font_size);

    // Lifecycle bookkeeping.
    session_map::pump_all(&mut d.sess);
    actions::auto_degrade(&mut d.st, &mut d.sess, &mut d.dirty);
    if d.st.win().is_some_and(|w| w.tree.tabs.is_empty()) && !d.ui.quitting {
        actions::do_new_tab(&mut d.st, &mut d.sess, PaneKind::Local, &mut d.dirty);
    }
    actions::ensure_sessions(&d.st, &mut d.sess);
    // A gone shell closes its own pane (after EXIT_GRACE); closing the
    // last pane of THIS window removes it mid-pass - stop rendering then
    // rather than drawing the next window's tree into this viewport.
    let win_id = d.st.win().map(|w| w.id);
    actions::close_exited(&mut d.st, &mut d.sess, &mut d.ui, &mut d.dirty);
    if d.st.win().map(|w| w.id) != win_id {
        return;
    }

    let area = ui.available_rect_before_wrap();
    let Data {
        st,
        sess,
        ui: uist,
        dirty,
        ..
    } = d;
    // A Ctrl+drag pane move owns the pointer: dividers and raw pointer
    // routing stand down while the drop target is being picked.
    let pane_drag = st.win().and_then(|w| w.ui.pane_drag);
    let dragging = if pane_drag.is_some() {
        false
    } else {
        mouse::divider_interaction(ui, st, area, dirty)
    };
    if let Some(w) = st.win_mut() {
        w.tree.active_tab = w.tree.active_tab.min(w.tree.tabs.len().saturating_sub(1));
    }
    let tab = st.win().map(|w| w.tree.active_tab).unwrap_or(0);
    let focused = st
        .win()
        .and_then(|w| w.tree.tabs.get(tab))
        .map(|t| t.focused);
    // IME anchor defaults to none each frame: the focused pane's live
    // session (loop below) refreshes it, so a dead pane or an empty tree
    // leaves IME unanchored.
    if let Some(w) = st.win_mut() {
        w.ui.ime_cursor = None;
        w.ui.ime_pane = None;
    }
    let cursor_a = tokens::cursor_alpha(ctx.input(|i| i.time) as f32);
    let painter = ui.painter().clone();
    let rects = pane_rects(st, area);

    // Raw pointer routing (reporting / selection / wheel) over pane content
    // rects; suppressed while a divider drag or a pane drag owns the
    // pointer.
    if !dragging && pane_drag.is_none() {
        let content_rects: Vec<(PaneId, Rect)> = rects
            .iter()
            .map(|(p, lt)| (*p, grid::egui_rect(content_rect(*lt, PANE_HEADER_H))))
            .collect();
        pointer::handle(&ctx, &content_rects, st, sess, cell.h, dirty);
    }

    for &(pane, lt) in &rects {
        let full = grid::egui_rect(lt);
        if full.width() < 4.0 || full.height() < 4.0 {
            continue;
        }
        let header = Rect::from_min_size(full.min, egui::vec2(full.width(), PANE_HEADER_H));
        let content = grid::egui_rect(content_rect(lt, PANE_HEADER_H));

        pane_header::show(ui, header, pane, tab, st, sess, uist, &pal, dirty);

        let fallback = crate::state::new_pane_meta(PaneKind::Local);
        let meta = st.panes.get(&pane).unwrap_or(&fallback);
        // No session at all (spawn backoff) or the child exited.
        let dead = sess.map.get(&pane).is_none_or(|s| s.exit.is_some());
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
                        font_size,
                        cursor_alpha: if Some(pane) == focused { cursor_a } else { 0.0 },
                        opacity: st.settings.opacity,
                    },
                );
                // Scrollback review indicator over the grid, right edge.
                if Some(pane) == focused && !vt_pane::viewport::pinned(s) {
                    if let Some(geo) = vt_pane::viewport::geometry(s) {
                        draw_viewport_bar(
                            &painter,
                            content,
                            geo,
                            // Quiet scrollbar: thumb/track derived from the
                            // pane background, no accent.
                            colors::with_opacity(
                                colors::to_c32(colors::mix(pal.background, pal.foreground, 0.22)),
                                st.settings.opacity,
                            ),
                            colors::with_opacity(
                                colors::to_c32(colors::mix(pal.background, pal.foreground, 0.06)),
                                st.settings.opacity,
                            ),
                        );
                    }
                }
                // IME anchor + composition overlay for the focused pane:
                // the popup anchors at the live cursor cell and any
                // in-flight preedit paints right above it.
                if Some(pane) == focused {
                    if let Some(anchor) = grid::cursor_rect(content, &fr, cell) {
                        let preedit = st.win().and_then(|w| w.ui.ime.clone());
                        if let Some(w) = st.win_mut() {
                            w.ui.ime_cursor = Some(anchor);
                            w.ui.ime_pane = Some(pane);
                        }
                        if let Some(text) = preedit.filter(|t| !t.is_empty()) {
                            preedit::paint(
                                &painter,
                                anchor,
                                &text,
                                font_size,
                                colors::to_c32(pal.foreground),
                                colors::to_c32(pal.block_highlight),
                            );
                        }
                    }
                }
            }
            _ => {
                // A corpse without an exit code yet is a spawn in backoff,
                // not a click-to-close candidate.
                let msg = match session_map::exit_code(sess, pane) {
                    Some(code) => format!("process exited (code {code}) - click to close"),
                    None => "no session - respawn from the context menu".to_string(),
                };
                grid::draw_dead(
                    &painter,
                    content,
                    colors::with_opacity(colors::effective_bg(&pal, meta), st.settings.opacity),
                    colors::to_c32(colors::title_text(&pal)),
                    &msg,
                );
            }
        }

        // A dead pane still carries its child's DEC mouse modes; they must
        // not suppress interaction with the corpse. Live mouse-grabbing
        // apps own right-clicks; only then is the context menu hidden.
        let tracking = !dead
            && sess
                .map
                .get(&pane)
                .map(vt_pane::mouse::is_mouse_tracking)
                .unwrap_or(false);
        if !dragging {
            let resp = mouse::pane_interact(ui, content, pane, st, dirty);
            if dead && resp.clicked() {
                actions::do_close_pane(st, sess, uist, tab, pane, dirty);
                continue;
            }
            if !tracking {
                resp.context_menu(|menu| {
                    pane_header::menu(menu, pane, tab, st, sess, uist, dirty);
                });
            }
        }

        let zoomed = st.win().is_some_and(|w| w.ui.zoom);
        if Some(pane) == focused && !zoomed {
            // Card focus: a 1.5px accent stroke plus a 2px accent bar on
            // the header's left edge (Warp/Ghostty convention) - quieter
            // than the old full-ring hard stroke.
            painter.rect_stroke(
                full,
                CARD_RADIUS,
                Stroke::new(1.5, colors::to_c32(pal.block_highlight)),
                StrokeKind::Inside,
            );
            painter.rect_filled(
                Rect::from_min_size(full.min, egui::vec2(2.0, PANE_HEADER_H)),
                1.0,
                colors::to_c32(pal.block_highlight),
            );
        } else if !zoomed {
            // Unfocused panes read as quiet cards separated from their
            // neighbours by a hairline seam.
            painter.rect_stroke(
                full,
                CARD_RADIUS,
                Stroke::new(1.0, colors::to_c32(colors::hairline(&pal))),
                StrokeKind::Inside,
            );
        }
    }

    // Divider strips over the background (non-zoom only): a quiet chrome
    // field with a 2px rounded-cap grab handle in the middle. Geometry
    // and hit-testing live in mouse.rs; this is paint only.
    if !st.win().is_some_and(|w| w.ui.zoom) {
        if let Some(tabref) = st.win().and_then(|w| w.tree.tabs.get(tab)) {
            let hover_key = ui.input(|i| i.pointer.hover_pos()).and_then(|p| {
                mouse::find_divider(tabref, grid::lt_rect(area), DIVIDER_W, p, 2.0)
                    .map(|h| (h.pane, h.level))
            });
            for h in mouse::dividers(tabref, grid::lt_rect(area), DIVIDER_W) {
                let s = h.strip;
                painter.rect_filled(
                    s,
                    0.0,
                    colors::with_opacity(
                        colors::to_c32(colors::chrome_bg(&pal)),
                        st.settings.opacity,
                    ),
                );
                // Handle brightens from the resting divider step to a
                // clear hover step while the pointer is on the strip.
                let resting = hover_key != Some((h.pane, h.level));
                let handle_col = colors::mix(
                    pal.background,
                    pal.foreground,
                    if resting { 0.13 } else { 0.22 },
                );
                let handle_col =
                    colors::with_opacity(colors::to_c32(handle_col), st.settings.opacity);
                // Tall strip = vertical split: the handle runs along y;
                // otherwise it is a horizontal strip: along x.
                let along = |len: f32| {
                    if s.height() >= s.width() {
                        Rect::from_center_size(s.center(), egui::vec2(2.0, len))
                    } else {
                        Rect::from_center_size(s.center(), egui::vec2(len, 2.0))
                    }
                };
                let span = if s.height() >= s.width() {
                    s.height()
                } else {
                    s.width()
                };
                let handle = along((span * 0.35).clamp(8.0, 64.0));
                painter.rect_filled(handle, 1.0, handle_col);
            }
        }
    }

    // Ctrl+drag pane move: refresh the drop target from the hovered pane
    // every frame (full pane rects - a drop may hover a header too) and
    // execute on release. Painted last so the overlay sits above panes
    // and dividers.
    if let Some(pd) = pane_drag {
        let pos = ui.input(|i| i.pointer.interact_pos());
        let new_target = pos.and_then(|p| {
            rects
                .iter()
                .rev()
                .find(|(id, r)| *id != pd.pane && grid::egui_rect(*r).contains(p))
                .map(|(id, r)| (*id, layout_tree::zone_for(r, p.x, p.y)))
        });
        if let Some(w) = st.win_mut() {
            w.ui.pane_drag = Some(crate::state::PaneDrag {
                pane: pd.pane,
                target: new_target,
            });
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            if let Some((target, zone)) = new_target {
                actions::do_move_pane(st, tab, pd.pane, target, zone, dirty);
            }
            if let Some(w) = st.win_mut() {
                w.ui.pane_drag = None;
            }
        } else if let Some((target, zone)) = new_target {
            let full = rects
                .iter()
                .find(|(id, _)| *id == target)
                .map(|(_, r)| grid::egui_rect(*r));
            let src = rects
                .iter()
                .find(|(id, _)| *id == pd.pane)
                .map(|(_, r)| grid::egui_rect(*r));
            if let (Some(full), Some(src)) = (full, src) {
                let preview = grid::egui_rect(layout_tree::zone_rect(
                    &grid::lt_rect(full),
                    zone,
                    st.settings.split_ratio,
                ));
                dropzone::paint(&painter, &pal, src, full, preview);
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        }
    }

    // Terminals stream output continuously.
    ctx.request_repaint_after(Duration::from_millis(50));
}
