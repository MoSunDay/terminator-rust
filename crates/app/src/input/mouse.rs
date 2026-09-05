//! Mouse interactions: split-divider dragging and pane focus.

use egui::{CursorIcon, Pos2, Rect, Sense, Ui};
use layout_tree::{
    set_ratio_at_level, split_rect_gapped, Axis, Node, PaneId, Rect as LRect, Tab, MAX_RATIO,
    MIN_RATIO,
};

use crate::state::{AppState, DragState};

/// Divider half-width for pointer hit-testing.
pub const DIVIDER_TOL: f32 = 4.0;

/// A divider line of a tab layout, tied to the split it belongs to.
#[derive(Debug, Clone, Copy)]
pub struct DividerHit {
    /// Leftmost pane of the split's first subtree (path key for set_ratio_at_level).
    pub pane: PaneId,
    /// Level of this split along the root->pane path (0 = outermost).
    pub level: usize,
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
/// `depth` counts split levels from the root (0 = outermost).
fn collect_dividers(node: &Node, area: LRect, gap: f32, depth: usize, out: &mut Vec<DividerHit>) {
    if let Node::Split {
        axis,
        ratio,
        first,
        second,
    } = node
    {
        let (a, b) = split_rect_gapped(area, *axis, *ratio, gap);
        let bounds = to_e(area);
        let strip = match axis {
            Axis::Horizontal => {
                Rect::from_min_size(egui::pos2(area.x, a.y + a.h), egui::vec2(area.w, gap))
            }
            Axis::Vertical => {
                Rect::from_min_size(egui::pos2(a.x + a.w, area.y), egui::vec2(gap, area.h))
            }
        };
        if let Some(pane) = first_pane(first) {
            out.push(DividerHit {
                pane,
                level: depth,
                axis: *axis,
                bounds,
                strip,
            });
        }
        collect_dividers(first, a, gap, depth + 1, out);
        collect_dividers(second, b, gap, depth + 1, out);
    }
}

/// All dividers of `tab` in screen coordinates.
pub fn dividers(tab: &Tab, area: LRect, gap: f32) -> Vec<DividerHit> {
    let mut out = Vec::new();
    collect_dividers(&tab.root, area, gap, 0, &mut out);
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
    dirty: &mut bool,
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
                    .find(|h| h.pane == d.pane && h.level == d.level)
                    .or_else(|| {
                        find_divider(&tab_cloned, lt_area, gap, pos, 32.0)
                            .filter(|h| h.pane == d.pane && h.level == d.level)
                    })
                {
                    let ratio = drag_ratio(&hit, pos, gap);
                    if let Some(t) = st.tree.tabs.get_mut(d.tab) {
                        if set_ratio_at_level(&mut t.root, d.pane, d.level, ratio) {
                            *dirty = true;
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
        *drag = Some(DragState {
            tab: tab_idx,
            pane: hit.pane,
            level: hit.level,
        });
        return true;
    }
    false
}

/// Focus-on-click layer for a pane region.
pub fn pane_interact(
    ui: &mut Ui,
    rect: Rect,
    pane: PaneId,
    st: &mut AppState,
    dirty: &mut bool,
) -> egui::Response {
    let resp = ui.interact(rect, egui::Id::new("pane").with(pane), Sense::click());
    if resp.clicked() {
        if let Some(t) = st.tree.tabs.get_mut(st.tree.active_tab) {
            if t.focused != pane {
                t.focused = pane;
                *dirty = true; // focus is persisted; save it
            }
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
        let area = LRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let ds = dividers(&tab, area, 4.0);
        assert_eq!(ds.len(), 2, "{ds:?}");
        let mut levels: Vec<usize> = ds.iter().map(|d| d.level).collect();
        levels.sort_unstable();
        assert_eq!(levels, vec![0, 1], "root split = 0, inner split = 1");
    }

    #[test]
    fn hit_vertical_divider_and_ratio() {
        let tab = three_pane_tab();
        let area = LRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        // The vertical divider sits at x=48..52, y=0..48 (pane1 top half).
        let pos = Pos2::new(50.0, 10.0);
        let hit = find_divider(&tab, area, 4.0, pos, DIVIDER_TOL);
        assert!(hit.is_some());
        let hit = hit.unwrap_or(DividerHit {
            pane: 0,
            level: 0,
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
            level: 0,
            axis: Axis::Vertical,
            bounds: Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0)),
            strip: Rect::ZERO,
        };
        assert!((drag_ratio(&hit, Pos2::new(-50.0, 0.0), 4.0) - 0.05).abs() < 1e-4);
        assert!((drag_ratio(&hit, Pos2::new(500.0, 0.0), 4.0) - 0.95).abs() < 1e-4);
    }

    #[test]
    fn level_targeting_selects_the_dragged_split() {
        let tab = three_pane_tab();
        let area = LRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let gap = 4.0;
        let mut root = tab.root;

        // Root split (horizontal, outermost): moving its ratio to 0.8 must
        // push the horizontal divider toward the bottom...
        assert!(set_ratio_at_level(&mut root, 1, 0, 0.8));
        let ds = dividers(
            &Tab {
                title: String::new(),
                root: root.clone(),
                focused: 1,
            },
            area,
            gap,
        );
        let h = ds
            .iter()
            .find(|d| d.axis == Axis::Horizontal)
            .unwrap_or(&ds[0]);
        assert!(
            h.strip.top() > 0.7 * area.h && h.strip.top() < 0.85 * area.h,
            "h divider at {}",
            h.strip.top()
        );
        let v = ds
            .iter()
            .find(|d| d.axis == Axis::Vertical)
            .unwrap_or(&ds[0]);
        assert!(
            (v.strip.left() - 48.0).abs() < 1.0,
            "v divider at {}",
            v.strip.left()
        );

        // ...while the inner vertical split (level 1) stays at ~48 until
        // targeted directly.
        assert!(set_ratio_at_level(&mut root, 1, 1, 0.3));
        let ds = dividers(
            &Tab {
                title: String::new(),
                root,
                focused: 1,
            },
            area,
            gap,
        );
        let v = ds
            .iter()
            .find(|d| d.axis == Axis::Vertical)
            .unwrap_or(&ds[0]);
        assert!(
            (v.strip.left() - 28.0).abs() < 1.0,
            "v divider at {}",
            v.strip.left()
        );
    }

    #[test]
    fn first_pane_walks_left() {
        let tab = three_pane_tab();
        assert_eq!(first_pane(&tab.root), Some(1));
    }
}
