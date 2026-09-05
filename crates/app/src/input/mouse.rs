//! Mouse interactions: split-divider dragging and pane focus.

use egui::{CursorIcon, Pos2, Rect, Sense, Ui};
use layout_tree::{
    set_parent_ratio, split_rect_gapped, Axis, Node, PaneId, Rect as LRect, Tab, MAX_RATIO,
    MIN_RATIO,
};

use crate::state::{AppState, DragState};

/// Divider half-width for pointer hit-testing.
pub const DIVIDER_TOL: f32 = 4.0;

/// A divider line of a tab layout, tied to the pane whose parent split owns
/// it (`set_parent_ratio` on that pane adjusts this divider).
#[derive(Debug, Clone, Copy)]
pub struct DividerHit {
    /// Any pane inside the split's first child (targets the innermost split).
    pub pane: PaneId,
    /// Split orientation.
    pub axis: Axis,
    /// Full bounds of the split (in screen coordinates).
    pub bounds: Rect,
    /// Divider strip (in screen coordinates).
    pub strip: Rect,
}

fn to_e(r: LRect) -> Rect {
    Rect::from_min_size(egui::pos2(r.x, r.y), egui::vec2(r.w, r.h))
}

/// First pane id in tree order (leftmost path down).
pub fn first_pane(node: &Node) -> Option<PaneId> {
    match node {
        Node::Pane { id } => Some(*id),
        Node::Split { first, .. } => first_pane(first),
    }
}

/// Recursively collect the divider strips of a pane tree laid out in `area`.
fn collect_dividers(node: &Node, area: LRect, gap: f32, out: &mut Vec<DividerHit>) {
    if let Node::Split { axis, ratio, first, second } = node {
        let (a, b) = split_rect_gapped(area, *axis, *ratio, gap);
        let bounds = to_e(area);
        let strip = match axis {
            Axis::Horizontal => Rect::from_min_size(
                egui::pos2(area.x, a.y + a.h),
                egui::vec2(area.w, gap),
            ),
            Axis::Vertical => Rect::from_min_size(
                egui::pos2(a.x + a.w, area.y),
                egui::vec2(gap, area.h),
            ),
        };
        if let Some(pane) = first_pane(first) {
            out.push(DividerHit { pane, axis: *axis, bounds, strip });
        }
        collect_dividers(first, a, gap, out);
        collect_dividers(second, b, gap, out);
    }
}

/// All dividers of `tab` in screen coordinates.
pub fn dividers(tab: &Tab, area: LRect, gap: f32) -> Vec<DividerHit> {
    let mut out = Vec::new();
    collect_dividers(&tab.root, area, gap, &mut out);
    out
}

/// Nearest divider under (or near) `pos`, within `tol` pixels.
pub fn find_divider(tab: &Tab, area: LRect, gap: f32, pos: Pos2, tol: f32) -> Option<DividerHit> {
    dividers(tab, area, gap)
        .into_iter()
        .filter(|d| {
            let grown = d.strip.expand(tol);
            grown.contains(pos)
        })
        .min_by_key(|d| {
            let c = d.strip.center();
            ((c.x - pos.x).abs() + (c.y - pos.y).abs()) as u64
        })
}

/// Ratio implied by dragging a divider to `pos`.
pub fn drag_ratio(hit: &DividerHit, pos: Pos2, gap: f32) -> f32 {
    let (len, at) = match hit.axis {
        Axis::Horizontal => (hit.bounds.height() - gap, pos.y - hit.bounds.top()),
        Axis::Vertical => (hit.bounds.width() - gap, pos.x - hit.bounds.left()),
    };
    if len <= f32::EPSILON {
        return MIN_RATIO;
    }
    (at / len).clamp(MIN_RATIO, MAX_RATIO)
}

/// One frame of divider hover/drag handling. Returns true while a drag is
/// active so callers can suppress pane interactions.
pub fn divider_interaction(
    ui: &mut Ui,
    st: &mut AppState,
    area: Rect,
    drag: &mut Option<DragState>,
) -> bool {
    let gap = crate::state::DIVIDER_W;
    let tab_idx = st.tree.active_tab;
    let Some(tab) = st.tree.tabs.get(tab_idx) else {
        *drag = None;
        return false;
    };
    let tab_cloned = tab.clone();
    let lt_area = crate::render::grid::lt_rect(area);

    if let Some(d) = drag.as_mut() {
        let down = ui.input(|i| i.pointer.any_down());
        let pos = ui.input(|i| i.pointer.hover_pos());
        if down {
            if let Some(pos) = pos {
                if let Some(hit) = dividers(&tab_cloned, lt_area, gap)
                    .into_iter()
                    .find(|h| h.pane == d.pane)
                    .or_else(|| find_divider(&tab_cloned, lt_area, gap, pos, 32.0).filter(|h| h.pane == d.pane))
                {
                    let ratio = drag_ratio(&hit, pos, gap);
                    if let Some(t) = st.tree.tabs.get_mut(d.tab) {
                        if set_parent_ratio(&mut t.root, d.pane, ratio) {
                            return true;
                        }
                    }
                }
            }
        }
        *drag = None;
    }

    let Some(pos) = ui.input(|i| i.pointer.hover_pos()) else {
        return false;
    };
    let Some(hit) = find_divider(&tab_cloned, lt_area, gap, pos, DIVIDER_TOL) else {
        return false;
    };
    ui.ctx().set_cursor_icon(match hit.axis {
        Axis::Vertical => CursorIcon::ResizeHorizontal,
        Axis::Horizontal => CursorIcon::ResizeVertical,
    });
    let resp = ui.interact(
        hit.strip,
        egui::Id::new("divider").with(hit.pane),
        Sense::drag(),
    );
    if resp.drag_started_by(egui::PointerButton::Primary) {
        *drag = Some(DragState { tab: tab_idx, pane: hit.pane });
        return true;
    }
    false
}

/// Focus-on-click layer for a pane region.
pub fn pane_interact(ui: &mut Ui, rect: Rect, pane: PaneId, st: &mut AppState) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::new("pane").with(pane), Sense::click());
    if resp.clicked() {
        if let Some(t) = st.tree.tabs.get_mut(st.tree.active_tab) {
            t.focused = pane;
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout_tree::split_pane;
    use layout_tree::tabs::new_tree;

    fn three_pane_tab() -> Tab {
        let mut tree = new_tree("main");
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Horizontal), Some(2));
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Vertical), Some(3));
        tree.tabs.remove(0)
    }

    #[test]
    fn finds_both_dividers() {
        let tab = three_pane_tab();
        let area = LRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let ds = dividers(&tab, area, 4.0);
        assert_eq!(ds.len(), 2, "{ds:?}");
    }

    #[test]
    fn hit_vertical_divider_and_ratio() {
        let tab = three_pane_tab();
        let area = LRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        // The vertical divider sits at x=48..52, y=0..48 (pane1 top half).
        let pos = Pos2::new(50.0, 10.0);
        let hit = find_divider(&tab, area, 4.0, pos, DIVIDER_TOL);
        assert!(hit.is_some());
        let hit = hit.unwrap_or(DividerHit {
            pane: 0,
            axis: Axis::Vertical,
            bounds: Rect::ZERO,
            strip: Rect::ZERO,
        });
        assert_eq!(hit.axis, Axis::Vertical);
        let r = drag_ratio(&hit, Pos2::new(72.0, 10.0), 4.0);
        assert!((r - 0.75).abs() < 0.01, "ratio {r}");
    }

    #[test]
    fn ratio_clamped() {
        let hit = DividerHit {
            pane: 1,
            axis: Axis::Vertical,
            bounds: Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0)),
            strip: Rect::ZERO,
        };
        assert!((drag_ratio(&hit, Pos2::new(-50.0, 0.0), 4.0) - 0.05).abs() < 1e-4);
        assert!((drag_ratio(&hit, Pos2::new(500.0, 0.0), 4.0) - 0.95).abs() < 1e-4);
    }

    #[test]
    fn first_pane_walks_left() {
        let tab = three_pane_tab();
        assert_eq!(first_pane(&tab.root), Some(1));
    }
}
