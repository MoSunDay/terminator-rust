//! Tab chips: ONE per-frame metadata pass (`measure`), chip drawing and
//! the strip's scroll/dwell math. Split out of `ui/tabs.rs` so the bar
//! itself only allocates, paints and routes interactions: today both the
//! overflow-span math and the paint loop laid out every chip label and
//! re-scanned every tab's panes on their own.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Ui};
use layout_tree::{PaneId, Tab};
use theme::Palette;

use crate::attention;
use crate::render::colors::{self, to_c32};
use crate::render::tokens;
use crate::state::{self, AppState};
use crate::ui::chrome::Metrics;

/// How long a pane-drag must hover another tab's chip before the active
/// tab switches there (browser tab-drag dwell).
const DWELL_SECS: f64 = 0.4;
/// Chip silhouette: full pill - the bar bg matches the chip field, so the
/// rounding reads as a soft silhouette instead of a tab-flap.
pub(super) const CHIP_RADIUS: CornerRadius = CornerRadius {
    nw: tokens::R_MD,
    ne: tokens::R_MD,
    sw: tokens::R_MD,
    se: tokens::R_MD,
};

/// One tab chip's metadata for THIS frame: the strip's span math and the
/// paint loop both consume it, so every label is laid out and every badge
/// scanned exactly once per frame.
pub(super) struct ChipInfo {
    /// Chip identity for reorder drags + the rename editor (anchor pane).
    pub anchor: PaneId,
    /// Raw tab title (seeds the rename buffer on double-click).
    pub title: String,
    /// Laid-out label galley; `None` while the rename editor owns the slot.
    pub galley: Option<Arc<egui::text::Galley>>,
    /// Chip advance width in the strip flow.
    pub width: f32,
    /// Attention badge (notice in a background/unfocused tab) is shown.
    pub badged: bool,
    /// Right-hand badge reservation folded into `width`.
    pub badge_w: f32,
    /// The rename editor replaces this chip's label this frame.
    pub editing: bool,
}

/// Single metadata pass over the window's tabs: label galley, badge state,
/// chip width, anchor and rename state. Pure reads - allocates no widgets
/// and mutates nothing.
pub(super) fn measure(
    ui: &egui::Ui,
    st: &AppState,
    notices: &BTreeSet<PaneId>,
    window_focused: bool,
    m: &Metrics,
) -> Vec<ChipInfo> {
    let painter = ui.painter().clone();
    let count = st.win().map(|w| w.tree.tabs.len()).unwrap_or(0);
    let active_tab = st.win().map(|w| w.tree.active_tab).unwrap_or(0);
    let edit_anchor = st
        .win()
        .and_then(|w| w.ui.tab_edit.as_ref().map(|(a, _)| *a));
    let mut infos = Vec::with_capacity(count);
    for i in 0..count {
        let Some(tab) = st.win().and_then(|w| w.tree.tabs.get(i)) else {
            continue;
        };
        let anchor = state::tab_anchor(tab);
        let editing = edit_anchor.is_some_and(|a| a == anchor);
        let selected = i == active_tab;
        // A tab holding an unacknowledged notice keeps its badge unless it
        // is the active tab of the focused window (the user sees it).
        let badged = !editing && attention::tab_needs_badge(tab, notices, selected, window_focused);
        let badge_w = if badged { 18.0 * m.s } else { 0.0 };
        let label = chip_label(tab);
        let galley = if editing {
            None
        } else {
            Some(painter.layout_no_wrap(
                label.clone(),
                FontId::proportional(m.chip_font),
                Color32::PLACEHOLDER,
            ))
        };
        let width = match &galley {
            Some(g) => (m.chip_pad_x * 2.0 + g.size().x + m.close_w + badge_w).max(m.chip_min_w),
            None => m.chip_edit_w,
        };
        infos.push(ChipInfo {
            anchor,
            title: tab.title.clone(),
            galley,
            width,
            badged,
            badge_w,
            editing,
        });
    }
    infos
}

/// Chip label: pane count suffix once the tab holds more than one pane.
pub(super) fn chip_label(tab: &Tab) -> String {
    let panes = layout_tree::pane_count(&tab.root);
    if panes > 1 {
        format!("{} [{}]", tab.title, panes)
    } else {
        tab.title.clone()
    }
}

/// Ghost chip for the in-flight tab drag: selected-chip look pinned under
/// the pointer. The label centers (the real chip left-aligns to leave
/// room for its close button, which the ghost does not carry).
pub(super) fn ghost_chip(
    ui: &mut Ui,
    pal: &Palette,
    label: &str,
    left: f32,
    top: f32,
    w: f32,
    m: &Metrics,
) {
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
pub(super) fn dwell_step(
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
pub(super) fn clamp_scroll(scroll: f32, overflow: f32) -> f32 {
    if scroll.is_nan() || overflow.is_nan() || overflow <= 0.0 {
        return 0.0;
    }
    scroll.clamp(0.0, overflow)
}

/// Scroll offset that brings the `[left, right]` span (unscrolled chip
/// coordinates) inside a strip window `area_w` wide: reveal the left
/// edge, else pull back until the right edge shows, else keep.
pub(super) fn ensure_visible(scroll: f32, left: f32, right: f32, area_w: f32) -> f32 {
    if left < scroll {
        left
    } else if right > scroll + area_w {
        right - area_w
    } else {
        scroll
    }
}

/// Wheel delta converted to strip pixels for each unit egui reports.
pub(super) fn wheel_px(unit: egui::MouseWheelUnit, delta: egui::Vec2, step: f32) -> egui::Vec2 {
    match unit {
        egui::MouseWheelUnit::Line => delta * step,
        egui::MouseWheelUnit::Page => delta * (step * 10.0),
        egui::MouseWheelUnit::Point => delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chrome;
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
    /// `measure` is the frame's SINGLE metadata pass: anchor, rename mode,
    /// badge reservation and the exact width formula all come from it.
    #[test]
    fn measure_is_one_pass_over_the_tabs() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        let ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("chips-measure"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(600.0, 40.0),
            )),
        );
        let m = chrome::metrics(15.0);
        let mut st = crate::state::fresh_state();
        let anchor = state::tab_anchor(&st.windows[0].tree.tabs[0]);
        let no_notices = BTreeSet::new();

        // Steady state: a real galley, no badge, the shared width formula.
        let infos = measure(&ui, &st, &no_notices, true, &m);
        assert_eq!(infos.len(), 1);
        let info = &infos[0];
        assert_eq!(info.anchor, anchor);
        assert_eq!(info.title, "shell");
        assert!(!info.editing);
        assert!(!info.badged);
        assert_eq!(info.badge_w, 0.0);
        let galley = info.galley.as_ref().expect("label laid out");
        let want = (m.chip_pad_x * 2.0 + galley.size().x + m.close_w).max(m.chip_min_w);
        assert_eq!(info.width, want);

        // A notice in a selected-but-unfocused tab earns the badge column
        // and widens the chip by exactly that reservation.
        let notices: BTreeSet<PaneId> = [anchor].into_iter().collect();
        let infos = measure(&ui, &st, &notices, false, &m);
        assert!(infos[0].badged);
        assert_eq!(infos[0].badge_w, 18.0 * m.s);
        assert_eq!(infos[0].width, want + 18.0 * m.s);
        assert!(
            !measure(&ui, &st, &notices, true, &m)[0].badged,
            "the focused window's active tab is already read"
        );

        // Rename mode: the editor owns the slot, no label galley is built.
        st.windows[0].ui.tab_edit = Some((anchor, "buf".to_string()));
        let infos = measure(&ui, &st, &no_notices, true, &m);
        assert!(infos[0].editing);
        assert!(infos[0].galley.is_none());
        assert_eq!(infos[0].width, m.chip_edit_w);
    }
}
