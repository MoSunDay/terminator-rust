//! Drag-and-drop pane moves: drop-zone geometry and the tree surgery behind
//! the Ctrl+drag pane-move gesture.

use crate::rect::{split_rect, Axis, Rect};
use crate::tabs::remove_pane_node;
use crate::tree::{
    contains_pane, find_node_mut, pane_count, LayoutTree, Node, PaneId, MAX_RATIO, MIN_RATIO,
};

/// Side of a target pane that a dragged pane would land on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropZone {
    /// Left edge: new vertical split, dragged pane first.
    Left,
    /// Right edge: new vertical split, dragged pane second.
    Right,
    /// Top edge: new horizontal split, dragged pane first.
    Top,
    /// Bottom edge: new horizontal split, dragged pane second.
    Bottom,
    /// Center square: swap the two pane ids in place.
    Center,
}

/// Which drop zone of `rect` the point `(x, y)` falls into: a centered
/// 50%-width square is the swap zone, the rest resolves to the nearest edge
/// (half of the pane along that axis). Points outside `rect` clamp to the
/// nearest zone.
pub fn zone_for(rect: &Rect, x: f32, y: f32) -> DropZone {
    let w = if rect.w > 0.0 { rect.w } else { 1.0 };
    let h = if rect.h > 0.0 { rect.h } else { 1.0 };
    let dx = ((x - rect.x) / w).clamp(0.0, 1.0);
    let dy = ((y - rect.y) / h).clamp(0.0, 1.0);
    let ex = (dx - 0.5).abs();
    let ey = (dy - 0.5).abs();
    if ex <= 0.25 && ey <= 0.25 {
        DropZone::Center
    } else if ex > ey {
        if dx < 0.5 {
            DropZone::Left
        } else {
            DropZone::Right
        }
    } else if dy < 0.5 {
        DropZone::Top
    } else {
        DropZone::Bottom
    }
}

/// The region the dragged pane would occupy when dropped in `zone`: the edge
/// share of `rect` at split `ratio` (clamped to `MIN_RATIO..=MAX_RATIO`),
/// the whole rect for [`DropZone::Center`].
pub fn zone_rect(rect: &Rect, zone: DropZone, ratio: f32) -> Rect {
    let ratio = ratio.clamp(MIN_RATIO, MAX_RATIO);
    match zone {
        DropZone::Center => *rect,
        DropZone::Left => split_rect(*rect, Axis::Vertical, ratio).0,
        DropZone::Right => split_rect(*rect, Axis::Vertical, ratio).1,
        DropZone::Top => split_rect(*rect, Axis::Horizontal, ratio).0,
        DropZone::Bottom => split_rect(*rect, Axis::Horizontal, ratio).1,
    }
}

/// Move `pane` next to `target` inside tab `tab` of the tree: an edge zone
/// detaches the pane leaf and re-splits the target leaf with the dragged
/// pane on the chosen side; [`DropZone::Center`] swaps the two leaf ids in
/// place. Focuses the moved pane. Pure tree surgery — sessions and pane meta
/// are keyed by pane id and stay valid. Tab title and `active_tab` are
/// untouched. Returns `true` on change, `false` (tree unchanged) when the
/// tab index is out of range, `pane == target`, either id is missing from
/// the tab or the tab holds fewer than two panes.
pub fn move_pane_to_pane(
    tree: &mut LayoutTree,
    tab: usize,
    pane: PaneId,
    target: PaneId,
    zone: DropZone,
    ratio: f32,
) -> bool {
    if pane == target || tab >= tree.tabs.len() {
        return false;
    }
    let t = &tree.tabs[tab];
    if !contains_pane(&t.root, pane) || !contains_pane(&t.root, target) || pane_count(&t.root) < 2 {
        return false;
    }
    if zone == DropZone::Center {
        swap_leaf_ids(&mut tree.tabs[tab].root, pane, target);
    } else {
        // Detach the dragged pane. The pane-count guard above ensures the
        // target leaf survives in the remaining root.
        let old_root = std::mem::replace(&mut tree.tabs[tab].root, Node::Pane { id: 0 });
        match remove_pane_node(old_root, pane) {
            Some(rest) => tree.tabs[tab].root = rest,
            None => {
                // Only reachable when the dragged pane is the entire root,
                // which the guard above excludes; never leave the
                // placeholder pane id 0 behind.
                tree.tabs[tab].root = Node::Pane { id: pane };
                return false;
            }
        }
        let t = &mut tree.tabs[tab];
        match find_node_mut(&mut t.root, target) {
            Some(node) => {
                let old = node.clone();
                // Left/Top put the dragged pane in the `first` slot,
                // Right/Bottom in `second` (see `Axis` for the convention).
                let (axis, dragged_first) = match zone {
                    DropZone::Left => (Axis::Vertical, true),
                    DropZone::Right => (Axis::Vertical, false),
                    DropZone::Top => (Axis::Horizontal, true),
                    DropZone::Bottom => (Axis::Horizontal, false),
                    DropZone::Center => (Axis::Vertical, true),
                };
                let dragged = Node::Pane { id: pane };
                let (first, second) = if dragged_first {
                    (dragged, old)
                } else {
                    (old, dragged)
                };
                *node = Node::Split {
                    axis,
                    ratio: ratio.clamp(MIN_RATIO, MAX_RATIO),
                    first: Box::new(first),
                    second: Box::new(second),
                };
            }
            // Guarded by `contains_pane` before the detach.
            None => return false,
        }
    }
    tree.tabs[tab].focused = pane;
    true
}

/// Rewrites leaf id `a` to `b` and vice versa throughout `node`, leaving
/// the structure and split ratios untouched.
fn swap_leaf_ids(node: &mut Node, a: PaneId, b: PaneId) {
    match node {
        Node::Pane { id } => {
            if *id == a {
                *id = b;
            } else if *id == b {
                *id = a;
            }
        }
        Node::Split { first, second, .. } => {
            swap_leaf_ids(first, a, b);
            swap_leaf_ids(second, a, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::{new_tab, new_tree};
    use crate::tree::split_pane;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    fn square() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        }
    }

    /// Two panes side by side: v(1 | 2) at ratio 0.5.
    fn two_panes() -> LayoutTree {
        let mut tree = new_tree("main");
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Vertical), Some(2));
        tree
    }

    /// Three panes: v(1 | v(2 | 3)), both splits at ratio 0.5.
    fn three_panes() -> LayoutTree {
        let mut tree = two_panes();
        assert_eq!(split_pane(&mut tree, 0, 2, Axis::Vertical), Some(3));
        tree
    }

    #[test]
    fn zone_for_covers_center_and_all_edges() {
        let r = square();
        assert_eq!(zone_for(&r, 50.0, 50.0), DropZone::Center);
        assert_eq!(zone_for(&r, 30.0, 70.0), DropZone::Center);
        // Exact center-square boundary counts as Center.
        assert_eq!(zone_for(&r, 25.0, 50.0), DropZone::Center);
        assert_eq!(zone_for(&r, 10.0, 50.0), DropZone::Left);
        assert_eq!(zone_for(&r, 90.0, 50.0), DropZone::Right);
        assert_eq!(zone_for(&r, 50.0, 10.0), DropZone::Top);
        assert_eq!(zone_for(&r, 50.0, 90.0), DropZone::Bottom);
        // Ties resolve to the horizontal axis (top/bottom).
        assert_eq!(zone_for(&r, 10.0, 10.0), DropZone::Top);
        assert_eq!(zone_for(&r, 90.0, 90.0), DropZone::Bottom);
    }

    #[test]
    fn zone_for_clamps_outside_points_to_nearest_zone() {
        let r = square();
        assert_eq!(zone_for(&r, -1000.0, 50.0), DropZone::Left);
        assert_eq!(zone_for(&r, 1000.0, 50.0), DropZone::Right);
        assert_eq!(zone_for(&r, 50.0, -1000.0), DropZone::Top);
        assert_eq!(zone_for(&r, 50.0, 1000.0), DropZone::Bottom);
        assert_eq!(zone_for(&r, -1000.0, -1000.0), DropZone::Top);
        assert_eq!(zone_for(&r, 1000.0, 1000.0), DropZone::Bottom);
        // Degenerate rect: zero extents are treated as unit length.
        let flat = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        assert_eq!(zone_for(&flat, 0.6, 0.5), DropZone::Center);
    }

    #[test]
    fn zone_rect_takes_the_edge_share() {
        let r = square();
        let left = zone_rect(&r, DropZone::Left, 0.25);
        assert!(close(left.x, 0.0) && close(left.w, 25.0) && close(left.h, 100.0));
        let right = zone_rect(&r, DropZone::Right, 0.25);
        assert!(close(right.x, 25.0) && close(right.w, 75.0) && close(right.h, 100.0));
        let top = zone_rect(&r, DropZone::Top, 0.25);
        assert!(close(top.y, 0.0) && close(top.h, 25.0) && close(top.w, 100.0));
        let bottom = zone_rect(&r, DropZone::Bottom, 0.25);
        assert!(close(bottom.y, 25.0) && close(bottom.h, 75.0) && close(bottom.w, 100.0));
        assert_eq!(zone_rect(&r, DropZone::Center, 0.25), r);
    }

    #[test]
    fn zone_rect_clamps_ratio_to_bounds() {
        let r = square();
        let left = zone_rect(&r, DropZone::Left, 0.0);
        assert!(close(left.w, 100.0 * MIN_RATIO));
        let right = zone_rect(&r, DropZone::Right, 0.0);
        assert!(close(right.x, 100.0 * MIN_RATIO) && close(right.w, 100.0 * MAX_RATIO));
        let top = zone_rect(&r, DropZone::Top, 2.0);
        assert!(close(top.h, 100.0 * MAX_RATIO));
        let bottom = zone_rect(&r, DropZone::Bottom, 2.0);
        assert!(close(bottom.y, 100.0 * MAX_RATIO) && close(bottom.h, 100.0 * MIN_RATIO));
    }

    #[test]
    fn move_left_reorders_a_two_pane_split() {
        let mut tree = two_panes(); // v(1 | 2)
        assert!(move_pane_to_pane(&mut tree, 0, 2, 1, DropZone::Left, 0.4));
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Vertical,
                ratio: 0.4,
                first: Box::new(Node::Pane { id: 2 }),
                second: Box::new(Node::Pane { id: 1 }),
            }
        );
        assert_eq!(tree.tabs[0].focused, 2);
    }

    #[test]
    fn move_right_on_three_panes_collapses_the_old_split() {
        let mut tree = three_panes(); // v(1 | v(2 | 3))
        assert!(move_pane_to_pane(&mut tree, 0, 1, 3, DropZone::Right, 0.3));
        // Detaching pane 1 collapsed the old root split: pane 2 now hangs
        // directly off the new root, pane 1 nested right of pane 3.
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(Node::Pane { id: 2 }),
                second: Box::new(Node::Split {
                    axis: Axis::Vertical,
                    ratio: 0.3,
                    first: Box::new(Node::Pane { id: 3 }),
                    second: Box::new(Node::Pane { id: 1 }),
                }),
            }
        );
        assert_eq!(pane_count(&tree.tabs[0].root), 3);
        assert_eq!(tree.tabs[0].focused, 1);
    }

    #[test]
    fn top_and_bottom_moves_stack_horizontally() {
        let mut tree = two_panes(); // v(1 | 2)
        assert!(move_pane_to_pane(&mut tree, 0, 2, 1, DropZone::Top, 0.5));
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Horizontal,
                ratio: 0.5,
                first: Box::new(Node::Pane { id: 2 }),
                second: Box::new(Node::Pane { id: 1 }),
            }
        );
        let mut tree = two_panes();
        assert!(move_pane_to_pane(
            &mut tree,
            0,
            1,
            2,
            DropZone::Bottom,
            0.25
        ));
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Horizontal,
                ratio: 0.25,
                first: Box::new(Node::Pane { id: 2 }),
                second: Box::new(Node::Pane { id: 1 }),
            }
        );
        assert_eq!(tree.tabs[0].focused, 1);
    }

    #[test]
    fn center_swap_exchanges_ids_in_place() {
        let mut tree = three_panes(); // v(1 | v(2 | 3))
        assert!(move_pane_to_pane(&mut tree, 0, 1, 3, DropZone::Center, 0.9));
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(Node::Pane { id: 3 }),
                second: Box::new(Node::Split {
                    axis: Axis::Vertical,
                    ratio: 0.5,
                    first: Box::new(Node::Pane { id: 2 }),
                    second: Box::new(Node::Pane { id: 1 }),
                }),
            }
        );
        assert_eq!(tree.tabs[0].focused, 1);
    }

    #[test]
    fn invalid_moves_are_rejected_unchanged() {
        let mut tree = three_panes();
        let before = tree.tabs[0].root.clone();
        assert!(!move_pane_to_pane(&mut tree, 0, 1, 1, DropZone::Left, 0.5));
        assert!(!move_pane_to_pane(&mut tree, 0, 1, 99, DropZone::Left, 0.5));
        assert!(!move_pane_to_pane(&mut tree, 0, 99, 1, DropZone::Left, 0.5));
        assert!(!move_pane_to_pane(&mut tree, 5, 1, 2, DropZone::Left, 0.5));
        assert_eq!(tree.tabs[0].root, before);
        assert_eq!(tree.tabs[0].focused, 3);
        assert_eq!(tree.active_tab, 0);
        // Other tabs are never touched.
        new_tab(&mut tree, "second");
        assert!(!move_pane_to_pane(
            &mut tree,
            0,
            1,
            4,
            DropZone::Center,
            0.5
        ));
        assert_eq!(tree.tabs[1].root, Node::Pane { id: 4 });
    }
}
