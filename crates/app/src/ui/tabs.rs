//! Chrome top bar: custom-drawn tab chip row + anchored edge cells.

use egui::{pos2, vec2, Align2, CornerRadius, FontId, Id, Key, Rect, Sense, TextEdit, Ui};
use layout_tree::Tab;
use theme::Palette;

use crate::actions;
use crate::actions::winops;
use crate::render::colors::{self, to_c32};
use crate::render::tokens;
use crate::state::{self, AppState, Data, WindowState};
use crate::ui::chrome::{self, Metrics};
use crate::ui::{tabs_widgets, xdrag};

/// How long a pane-drag must hover another tab's chip before the active
/// tab switches there (browser tab-drag dwell).
const DWELL_SECS: f64 = 0.4;
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
    let m = chrome::metrics(d.st.settings.font_size);
    tab_row(ui, d, &pal, &m);
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

fn tab_row(ui: &mut Ui, d: &mut Data, pal: &Palette, m: &Metrics) {
    let row = ui.max_rect();
    // Borderless window: dragging the bare chrome (not a chip or button)
    // moves the window - this IS the title-bar replacement. Registered
    // first so widgets added later (and thus on top) keep their clicks;
    // drags fall through to this background.
    // A chip reorder drag owns the pointer: no window move then.
    let chip_reorder = d.st.win().is_some_and(|w| w.ui.tab_drag.is_some());
    // click_and_drag: double-click-to-maximize needs the click half of
    // the sense; the drag half keeps the window-move gesture.
    // Sense::CLICK | DRAG (not Sense::click_and_drag): the FOCUSABLE bit
    // would let a bare Tab hand keyboard focus to the chrome and swallow
    // all pane keys (egui 0.36 Sense::click* constructors are focusable).
    let drag = ui.interact(row, Id::new("chrome_drag"), Sense::CLICK | Sense::DRAG);
    if drag.double_clicked() {
        tabs_widgets::send_toggle_enlarge(ui.ctx());
    } else if !chip_reorder {
        // Window move only after a REAL drag (see ui::arm_window_drag):
        // micro-drift clicks must stay plain clicks so the WM keeps
        // activating the window on click.
        let dragging = drag.dragged_by(egui::PointerButton::Primary);
        let delta = drag.drag_delta().length();
        let down = ui.input(|i| i.pointer.any_down());
        let fresh = ui.input(|i| i.pointer.any_pressed());
        if let Some(w) = d.st.win_mut() {
            crate::ui::arm_window_drag(
                ui.ctx(),
                &mut w.ui.window_move_armed,
                &mut w.ui.window_move_travel,
                dragging,
                delta,
                down,
                fresh,
            );
        }
    }
    ui.add_space(m.row_inset);
    // Chip slot centers (center_x, tab index) plus the first chip's top:
    // consumed by the ghost/reorder pass after the row is allocated.
    let mut centers: Vec<(f32, usize)> = Vec::new();
    let mut chip_top: Option<f32> = None;
    // Chip rects (rect, tab index) for the pane-drag dwell pass: hover
    // hit-testing on the allocated row after the chip loop. Values are
    // the SCROLLED (visible) rects.
    let mut chip_rects: Vec<(Rect, usize)> = Vec::new();

    // --- Chip-strip overflow scrolling -----------------------------------
    // Precompute the unscrolled chip spans (relative to row.left()) with
    // the same width formula the paint loop uses, so the strip only
    // scrolls when the tabs really overflow the space left of the pinned
    // chrome.
    let painter = ui.painter().clone();
    let count = d.st.win().map(|w| w.tree.tabs.len()).unwrap_or(0);
    let mut spans: Vec<(f32, f32)> = Vec::with_capacity(count); // (left, right) rel
    let mut flow = 0.0f32;
    for i in 0..count {
        let Some(tab) = d.st.win().and_then(|w| w.tree.tabs.get(i)) else {
            continue;
        };
        let editing = d.st.win().is_some_and(|w| {
            w.ui.tab_edit
                .as_ref()
                .is_some_and(|(a, _)| *a == state::tab_anchor(tab))
        });
        let w = if editing {
            m.chip_edit_w
        } else {
            let galley = painter.layout_no_wrap(
                chip_label(tab),
                FontId::proportional(m.chip_font),
                egui::Color32::PLACEHOLDER,
            );
            (m.chip_pad_x * 2.0 + galley.size().x + m.close_w).max(m.chip_min_w)
        };
        spans.push((flow, flow + w));
        flow += w + m.chip_gap;
    }
    let total_w = flow - if spans.is_empty() { 0.0 } else { m.chip_gap };
    let strip = Rect::from_min_max(
        pos2(row.left(), row.top() + m.row_inset),
        pos2(row.right() - m.reserve, row.bottom()),
    );
    // Cross-window tab drags: publish this window's strip in SCREEN
    // points every pass (strip_hit consumes it while a chip drag is
    // live), and tint this strip when ANOTHER window's drag hovers it.
    let win_id = d.st.win().map(|w| w.id).unwrap_or(0);
    xdrag::publish(ui, d, strip);
    if d.ui.xdrag.as_ref().and_then(|x| x.over) == Some(win_id) {
        xdrag::paint_strip_tint(ui, pal, strip);
    }
    let strip_w = (strip.right() - row.left()).max(0.0);
    let overflow = (total_w - strip_w).max(0.0);
    let mut scroll = d.st.win().map(|w| w.ui.tab_scroll).unwrap_or(0.0);
    // Wheel over the strip scrolls the chips (horizontal tracks px.x,
    // the usual vertical wheel drives the same axis).
    if ui
        .input(|i| i.pointer.interact_pos())
        .is_some_and(|p| strip.contains(p))
    {
        let px = ui.input(|i| {
            i.events.iter().fold(egui::Vec2::ZERO, |acc, e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => {
                    acc + wheel_px(*unit, *delta, m.wheel_step)
                }
                _ => acc,
            })
        });
        scroll = clamp_scroll(scroll + (px.x - px.y), overflow);
    }
    // Auto-follow: switching tabs snaps the strip just enough to reveal
    // the active chip - once per switch, so manual scrolling wins between
    // them.
    let active_tab = d.st.win().map(|w| w.tree.active_tab).unwrap_or(0);
    if let Some(w) = d.st.win_mut() {
        if w.ui.tab_scroll_tab != active_tab {
            w.ui.tab_scroll_tab = active_tab;
            if let Some(&(l, r)) = spans.get(active_tab) {
                scroll = ensure_visible(scroll, l, r, strip_w).clamp(0.0, overflow);
            }
        }
        w.ui.tab_scroll = scroll;
    }
    // Chip flow clip: off-view chips neither paint nor receive clicks
    // (egui hit-tests rect ∩ clip), and their ink never bleeds under the
    // pinned right-side chrome.
    let strip_span = Rect::from_min_max(
        pos2(row.left(), row.top()),
        pos2(row.right() - m.reserve, row.bottom()),
    );
    ui.horizontal(|ui| {
        let saved_clip = ui.clip_rect();
        ui.set_clip_rect(saved_clip.intersect(strip_span));
        ui.spacing_mut().item_spacing.x = m.chip_gap;
        let Data {
            st,
            sess,
            ui: uist,
            dirty,
            ..
        } = d;
        let mut close_tab: Option<usize> = None;
        let count = st.win().map(|w| w.tree.tabs.len()).unwrap_or(0);
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
                        .desired_width(110.0 * m.s)
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
                FontId::proportional(m.chip_font),
                egui::Color32::PLACEHOLDER,
            );
            let w = (m.chip_pad_x * 2.0 + galley.size().x + m.close_w).max(m.chip_min_w);
            if tab_drag.is_some_and(|td| td.anchor == anchor) {
                // The dragged chip's slot stays an empty gap: the other
                // chips shift around it live while the ghost follows the
                // pointer (painted after the row, above the edge cells).
                let (rect, _) = ui.allocate_exact_size(vec2(w, m.chip_h), Sense::hover());
                let vrect = rect.translate(vec2(-scroll, 0.0));
                centers.push((vrect.center().x, i));
                chip_top = chip_top.or(Some(vrect.top()));
                chip_rects.push((vrect, i));
                continue;
            }
            // Flow slot (unscrolled cursor advance) + the visible rect the
            // chip is actually painted/hit-tested at. Fully off-view chips
            // keep their bookkeeping entries (reorder centers, dwell
            // hit-rects) but skip interaction and painting.
            let (rect, _) = ui.allocate_exact_size(vec2(w, m.chip_h), Sense::hover());
            let vrect = rect.translate(vec2(-scroll, 0.0));
            centers.push((vrect.center().x, i));
            chip_top = chip_top.or(Some(vrect.top()));
            chip_rects.push((vrect, i));
            if vrect.right() <= strip.left() || vrect.left() >= strip.right() {
                continue;
            }
            // click_and_drag: egui disambiguates by pointer movement, so
            // click-to-switch / double-click rename / middle-click close
            // all keep working next to the reorder drag.
            let resp = ui.interact(
                vrect,
                Id::new("tab_chip").with(i),
                Sense::CLICK | Sense::DRAG,
            );
            // Chrome behavior: a drag also selects the tab. The latch is
            // keyed by the tab anchor, immune to the index shifts the
            // reorder itself causes.
            if tab_drag.is_none() && resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(px) = ui.input(|i| i.pointer.interact_pos()).map(|p| p.x) {
                    if let Some(w) = st.win_mut() {
                        w.ui.tab_drag = Some(state::TabDrag {
                            anchor,
                            grab_dx: px - vrect.left(),
                            w: vrect.width(),
                        });
                        w.tree.active_tab = i;
                        w.ui.zoom = false;
                    }
                    *dirty = true;
                }
            }
            if selected {
                painter.rect_filled(vrect, CHIP_RADIUS, to_c32(colors::tab_active(pal)));
                // Rounded-cap accent underline flush at the chip bottom:
                // this tab owns the content below. Inset clear of the pill
                // corners so the caps stay on the straight edge.
                let underline = Rect::from_min_max(
                    pos2(
                        vrect.left() + f32::from(tokens::R_MD) + 2.0,
                        vrect.bottom() - m.accent_h,
                    ),
                    pos2(
                        vrect.right() - f32::from(tokens::R_MD) - 2.0,
                        vrect.bottom(),
                    ),
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
                        vrect,
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
                    pos2(vrect.left() + m.chip_pad_x, vrect.top()),
                    pos2(vrect.right() - m.close_w, vrect.bottom()),
                ),
            );
            painter.galley(galley_rect.min, galley, text_col);

            // Close affordance: right-hand strip, revealed on hover/active.
            let close_rect = Rect::from_center_size(
                pos2(vrect.right() - m.close_w * 0.5, vrect.center().y),
                vec2(m.close_btn, m.close_btn),
            );
            let close_resp = ui.interact(close_rect, Id::new("tab_close").with(i), Sense::CLICK);
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
                    *dirty = true; // active_tab is persisted
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

        // Restore the clip before the pinned chrome widgets: they live
        // right of the strip and must not be clipped away.
        ui.set_clip_rect(saved_clip);
        ui.add_space(m.group_pad);
        // The trailing +/split group FOLLOWS the strip: 8px right of the
        // last chip (in scrolled coordinates), vertically centered on the
        // row. When the chips overflow, it parks flush left of the edge
        // cells so it never leaves the window.
        let group_left = (row.left() + total_w + m.group_pad - scroll)
            .clamp(row.left(), (row.right() - m.reserve).max(row.left()));
        let group_top = row.center().y - m.icon * 0.5;
        tabs_widgets::trailing_buttons(ui, pos2(group_left, group_top), st, sess, pal, dirty, m);
        tabs_widgets::edge_cells(ui, row.right(), st, m);
        // Pane-move cross-tab targeting: dwelling on another tab's chip for
        // a beat switches the active tab mid-drag (browser-style) — the
        // drop zones then come from THAT tab's panes and the pane
        // migrates there. Chip rects are translated vrects, so keep the
        // dwell ring clipped to the strip (it must not ride over the
        // pinned chrome when the hovered chip is half scrolled out).
        ui.set_clip_rect(saved_clip.intersect(strip_span));
        if let Some(pd) = st.win().and_then(|w| w.ui.pane_drag) {
            let now = ui.input(|i| i.time);
            let pos = ui.input(|i| i.pointer.interact_pos());
            let hover_hit =
                pos.and_then(|p| chip_rects.iter().copied().find(|(r, _)| r.contains(p)));
            let hover = hover_hit.map(|(_, i)| i);
            let active = st.win().map(|w| w.tree.active_tab).unwrap_or(0);
            let (dwell, switch) = dwell_step(hover, pd.dwell, active, now);
            // Drop-target affordance: the hovered chip of a NON-active tab
            // gets an accent ring while the dwell is pending.
            if let Some((rect, _)) = hover_hit.filter(|(_, i)| *i != active) {
                ui.painter().rect_stroke(
                    rect,
                    CHIP_RADIUS,
                    egui::Stroke::new(1.5 * m.s, to_c32(pal.block_highlight)),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(w) = st.win_mut() {
                if switch {
                    if let Some((_, i)) = hover_hit {
                        w.tree.active_tab = i;
                        w.ui.zoom = false;
                        w.ui.pane_drag = Some(state::PaneDrag { dwell: None, ..pd });
                        *dirty = true; // active_tab is persisted
                    }
                } else {
                    w.ui.pane_drag = Some(state::PaneDrag { dwell, ..pd });
                }
            }
        }
        ui.set_clip_rect(saved_clip);
    });
    // In-flight chip drag: live reorder + ghost chip. Runs after the row
    // closure so the ghost paints above the trailing buttons/edge cells;
    // the anchor pane closing mid-drag (impossible from the bar itself,
    // reachable via ctl) drops the drag. Cross-window: the pointer is
    // hit-tested against every OTHER window's published strip, a hovered
    // foreign strip suppresses the local live reorder, and the release
    // hands the tab to that window (sessions follow the pane ids).
    if let Some(td) = d.st.win().and_then(|w| w.ui.tab_drag) {
        let win_id = d.st.win().map(|w| w.id).unwrap_or(0);
        let cur = d.st.win().and_then(|w| {
            w.tree
                .tabs
                .iter()
                .position(|t| state::tab_anchor(t) == td.anchor)
        });
        let pos = ui.input(|i| i.pointer.interact_pos());
        let (over, polled_down) = xdrag::step(ui, d, td.anchor, win_id);
        // Release truth while the pointer is in ANOTHER window: this
        // window then receives no pointer events at all, so
        // i.pointer.primary_down() would never clear - the X11 poll
        // (position + button mask) answers. Event path = Wayland/tests.
        let held = match polled_down {
            Some(down) => down,
            None => pos.is_some() && ui.input(|i| i.pointer.primary_down()),
        };
        match cur {
            None => {
                if let Some(w) = d.st.win_mut() {
                    w.ui.tab_drag = None;
                }
                d.ui.xdrag = None;
                d.ui.pointer_screen = None;
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
                    if target != cur && over.is_none() {
                        if let Some(w) = d.st.win_mut() {
                            if layout_tree::move_tab(&mut w.tree, cur, target) {
                                d.dirty = true;
                            }
                        }
                    }
                    if held && over.is_none() {
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
                            ghost_chip(ui, pal, &label, ghost_left, top, td.w, m);
                        }
                    }
                }
                if !held {
                    if let Some(w) = d.st.win_mut() {
                        w.ui.tab_drag = None;
                    }
                    // Released over another window's strip: pure state
                    // surgery (the panes keep their live sessions), then
                    // raise the receiving window.
                    if let Some(dst) = over {
                        let vp = winops::viewport_of(&d.st, dst);
                        ui.ctx()
                            .send_viewport_cmd_to(vp, egui::ViewportCommand::Focus);
                        winops::do_move_tab_to_window(&mut d.st, &mut d.dirty, td.anchor, dst);
                    }
                    d.ui.xdrag = None;
                    d.ui.pointer_screen = None;
                }
            }
        }
    }
    // Cross-window drop affordance on the RECEIVING side: 2px accent
    // caret in the gap beside the chip nearest the incoming pointer.
    if d.ui.xdrag.as_ref().and_then(|x| x.over) == Some(win_id) {
        let ptr = xdrag::local_pointer(ui, d.ui.pointer_screen);
        xdrag::paint_caret(ui, pal, strip, &chip_rects, ptr, m.chip_gap);
    }
    ui.add_space(m.row_inset);
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
fn ghost_chip(ui: &mut Ui, pal: &Palette, label: &str, left: f32, top: f32, w: f32, m: &Metrics) {
    let painter = ui.painter().clone();
    let rect = Rect::from_min_size(pos2(left, top), vec2(w, m.chip_h));
    painter.rect_filled(rect, CHIP_RADIUS, to_c32(colors::tab_active(pal)));
    // Rounded-cap accent underline, inset like the real chip.
    let underline = Rect::from_min_max(
        pos2(
            rect.left() + f32::from(tokens::R_MD) + 2.0,
            rect.bottom() - m.accent_h,
        ),
        pos2(rect.right() - f32::from(tokens::R_MD) - 2.0, rect.bottom()),
    );
    painter.rect_filled(underline, 1.0, to_c32(pal.block_highlight));
    let galley = painter.layout_no_wrap(
        label.to_owned(),
        FontId::proportional(m.chip_font),
        egui::Color32::PLACEHOLDER,
    );
    let text_rect = Align2::CENTER_CENTER.align_size_within_rect(galley.size(), rect);
    painter.galley(text_rect.min, galley, to_c32(pal.foreground));
}

/// Hover-dwell state for the pane-drag tab switch: returns the new dwell
/// state and whether the active tab should switch to the hovered chip's
/// tab now. Arms (or keeps) the timer only while a NON-active chip is
/// hovered; anything else clears it.
fn dwell_step(
    hover: Option<usize>,
    dwell: Option<(usize, f64)>,
    active: usize,
    now: f64,
) -> (Option<(usize, f64)>, bool) {
    match hover {
        Some(i) if i != active => {
            // Same chip as before keeps the armed start time; hopping to
            // another chip restarts the clock.
            let armed = match dwell {
                Some((j, t)) if j == i => (i, t),
                _ => (i, now),
            };
            if now - armed.1 >= DWELL_SECS {
                (None, true)
            } else {
                (Some(armed), false)
            }
        }
        _ => (None, false),
    }
}

/// Clamp a chip-strip scroll offset into `0..=overflow`; NaN (never
/// allocated, corrupt state) reads as "no scroll".
fn clamp_scroll(scroll: f32, overflow: f32) -> f32 {
    if scroll.is_nan() || overflow.is_nan() || overflow <= 0.0 {
        return 0.0;
    }
    scroll.clamp(0.0, overflow)
}

/// Scroll offset that brings the `[left, right]` span (unscrolled chip
/// coordinates) inside a strip window `area_w` wide: reveal the left
/// edge, else pull back until the right edge shows, else keep.
fn ensure_visible(scroll: f32, left: f32, right: f32, area_w: f32) -> f32 {
    if left < scroll {
        left
    } else if right > scroll + area_w {
        right - area_w
    } else {
        scroll
    }
}

/// Wheel delta converted to strip pixels for each unit egui reports.
fn wheel_px(unit: egui::MouseWheelUnit, delta: egui::Vec2, step: f32) -> egui::Vec2 {
    match unit {
        egui::MouseWheelUnit::Line => delta * step,
        egui::MouseWheelUnit::Page => delta * (step * 10.0),
        egui::MouseWheelUnit::Point => delta,
    }
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

    #[test]
    fn dwell_step_arms_and_keeps_start_time() {
        // A fresh hover on another tab's chip arms the timer at `now`.
        let (dwell, switch) = dwell_step(Some(1), None, 0, 10.0);
        assert_eq!(dwell, Some((1, 10.0)));
        assert!(!switch, "not due yet");
        // Staying on the same chip keeps the ORIGINAL start time.
        let (dwell, switch) = dwell_step(Some(1), dwell, 0, 10.2);
        assert_eq!(dwell, Some((1, 10.0)));
        assert!(!switch);
    }

    #[test]
    fn dwell_step_switches_when_due() {
        let (dwell, switch) = dwell_step(Some(1), Some((1, 10.0)), 0, 10.5);
        assert_eq!(dwell, None, "the switch consumes the timer");
        assert!(switch);
    }

    #[test]
    fn dwell_step_restarts_on_chip_change() {
        let (dwell, _) = dwell_step(Some(2), Some((1, 10.0)), 0, 10.3);
        assert_eq!(dwell, Some((2, 10.3)), "hopping chips re-arms from now");
    }

    #[test]
    fn dwell_step_clears_on_active_tab_hover() {
        let (dwell, switch) = dwell_step(Some(0), Some((0, 10.0)), 0, 10.9);
        assert_eq!(dwell, None);
        assert!(!switch, "hovering the active tab never switches");
    }

    #[test]
    fn dwell_step_clears_without_hover() {
        let (dwell, switch) = dwell_step(None, Some((1, 10.0)), 0, 10.9);
        assert_eq!(dwell, None);
        assert!(!switch);
    }

    #[test]
    fn clamp_scroll_bounds_and_nan() {
        assert_eq!(clamp_scroll(-10.0, 100.0), 0.0);
        assert_eq!(clamp_scroll(50.0, 100.0), 50.0);
        assert_eq!(clamp_scroll(500.0, 100.0), 100.0);
        assert_eq!(clamp_scroll(f32::NAN, 100.0), 0.0);
        assert_eq!(clamp_scroll(50.0, f32::NAN), 0.0);
        assert_eq!(clamp_scroll(50.0, 0.0), 0.0, "no overflow = no scroll");
    }

    #[test]
    fn ensure_visible_branches() {
        // Chip starts left of the window: reveal its left edge.
        assert_eq!(ensure_visible(200.0, 50.0, 120.0, 300.0), 50.0);
        // Chip ends past the window's right edge: pull left until it fits.
        assert_eq!(ensure_visible(0.0, 50.0, 400.0, 300.0), 100.0);
        // Already inside: untouched.
        assert_eq!(ensure_visible(50.0, 80.0, 300.0, 300.0), 50.0);
    }

    #[test]
    fn wheel_px_units() {
        let d = egui::Vec2::new(1.0, -2.0);
        assert_eq!(wheel_px(egui::MouseWheelUnit::Point, d, 48.0), d);
        assert_eq!(
            wheel_px(egui::MouseWheelUnit::Line, d, 48.0),
            egui::Vec2::new(48.0, -96.0)
        );
        assert_eq!(
            wheel_px(egui::MouseWheelUnit::Page, d, 48.0),
            egui::Vec2::new(480.0, -960.0)
        );
    }
}
