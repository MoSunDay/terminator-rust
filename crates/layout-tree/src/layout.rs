//! Computing on-screen geometry for a tab's pane tree.

use crate::rect::{rect_contains, split_rect_gapped, Rect};
use crate::tree::{Node, PaneId, Tab};

/// Divider thickness used by helpers that do not receive one from the caller.
pub const DEFAULT_DIVIDER_W: f32 = 6.0;

/// Lays out every pane of `tab` inside `area` and returns `(pane_id, rect)`
/// pairs in tree order.
///
/// The returned rects are the **full** pane rects: they include the pane
/// header strip, which the app insets with [`content_rect`] before drawing
/// terminal content. `pane_header_h` is that strip's height, accepted here so
/// callers keep a single source of truth for it; the geometry itself does not
/// depend on it.
///
/// `divider_w` pixels of divider are inserted between the children of every
/// split: the first child gets `ratio * (major_len - divider_w)` of the
/// length, the divider occupies the sliver after it and the second child
/// gets the remainder.
pub fn layout_tab(
    tab: &Tab,
    area: Rect,
    pane_header_h: f32,
    divider_w: f32,
) -> Vec<(PaneId, Rect)> {
    // Rects deliberately include the header strip; the parameter is part of
    // the signature so callers and `content_rect` share one source of truth.
    let _ = pane_header_h;
    let mut out = Vec::new();
    layout_node(&tab.root, area, divider_w, &mut out);
    out
}

/// Recursive worker of [`layout_tab`].
fn layout_node(node: &Node, area: Rect, divider_w: f32, out: &mut Vec<(PaneId, Rect)>) {
    match node {
        Node::Pane { id } => out.push((*id, area)),
        Node::Split { axis, ratio, first, second } => {
            let (first_area, second_area) = split_rect_gapped(area, *axis, *ratio, divider_w);
            layout_node(first, first_area, divider_w, out);
            layout_node(second, second_area, divider_w, out);
        }
    }
}

/// Terminal-content area of a pane rect: the pane rect minus its header strip.
pub fn content_rect(pane: Rect, pane_header_h: f32) -> Rect {
    Rect {
        y: pane.y + pane_header_h,
        h: (pane.h - pane_header_h).max(0.0),
        ..pane
    }
}

/// The topmost pane of `tab` containing the point `(px, py)` when the tab is
/// laid out over `area` with [`DEFAULT_DIVIDER_W`].
///
/// Points that fall into a divider gap or outside `area` return `None`. On
/// rects sharing an edge, the pane earlier in tree order wins.
pub fn pane_at(tab: &Tab, area: Rect, px: f32, py: f32) -> Option<PaneId> {
    layout_tab(tab, area, 0.0, DEFAULT_DIVIDER_W)
        .into_iter()
        .find(|(_, r)| rect_contains(*r, px, py))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rect::{rect_area, Axis};
    use crate::tabs::new_tree;
    use crate::tree::{set_parent_ratio, split_pane};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// Same 3-pane shape as the other modules: pane 1 top-left, pane 3
    /// top-right, pane 2 across the bottom.
    fn three_panes() -> Tab {
        let mut tree = new_tree("main");
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Horizontal), Some(2));
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Vertical), Some(3));
        tree.tabs.remove(0)
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn layout_partitions_area() {
        let tab = three_panes();
        let area = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let rects = layout_tab(&tab, area, 10.0, 4.0);
        assert_eq!(rects.len(), 3);
        // Exact rects: divider 4px, ratio 0.5 everywhere.
        assert_eq!(rects[0], (1, Rect { x: 0.0, y: 0.0, w: 48.0, h: 48.0 }));
        assert_eq!(rects[1], (3, Rect { x: 52.0, y: 0.0, w: 48.0, h: 48.0 }));
        assert_eq!(rects[2], (2, Rect { x: 0.0, y: 52.0, w: 100.0, h: 48.0 }));
        // Panes stay inside the area and never overlap.
        for (_, r) in &rects {
            assert!(rect_contains(area, r.x, r.y));
            assert!(rect_contains(area, r.x + r.w, r.y + r.h));
        }
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                assert!(!overlaps(rects[i].1, rects[j].1));
            }
        }
        // Pane areas + both divider strips == total area.
        let panes: f32 = rects.iter().map(|(_, r)| rect_area(*r)).sum();
        let dividers = 4.0 * 100.0 + 4.0 * 48.0;
        assert!(close(panes + dividers, rect_area(area)));
    }

    #[test]
    fn layout_follows_ratio() {
        let mut tab = three_panes();
        assert!(set_parent_ratio(&mut tab.root, 1, 0.75));
        let area = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        let rects = layout_tab(&tab, area, 0.0, 4.0);
        assert_eq!(rects[0], (1, Rect { x: 0.0, y: 0.0, w: 72.0, h: 48.0 }));
        assert_eq!(rects[1], (3, Rect { x: 76.0, y: 0.0, w: 24.0, h: 48.0 }));
    }

    #[test]
    fn content_rect_insets_header() {
        let pane = Rect { x: 0.0, y: 52.0, w: 100.0, h: 48.0 };
        assert_eq!(
            content_rect(pane, 10.0),
            Rect { x: 0.0, y: 62.0, w: 100.0, h: 38.0 }
        );
        assert!(content_rect(Rect { x: 0.0, y: 0.0, w: 5.0, h: 2.0 }, 10.0).h <= 0.0);
    }

    #[test]
    fn pane_at_hits_panes_and_misses_dividers() {
        let tab = three_panes();
        let area = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 };
        assert_eq!(pane_at(&tab, area, 10.0, 10.0), Some(1));
        assert_eq!(pane_at(&tab, area, 90.0, 10.0), Some(3));
        assert_eq!(pane_at(&tab, area, 10.0, 90.0), Some(2));
        assert_eq!(pane_at(&tab, area, 99.9, 99.9), Some(2)); // closed far edge
        assert_eq!(pane_at(&tab, area, 50.0, 10.0), None); // vertical divider gap
        assert_eq!(pane_at(&tab, area, 50.0, 50.0), None); // horizontal divider gap
        assert_eq!(pane_at(&tab, area, -1.0, 0.0), None); // outside
    }
}
