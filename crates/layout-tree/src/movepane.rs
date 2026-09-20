//! Drag-and-drop pane moves: drop-zone geometry and the tree surgery behind
//! the Ctrl+drag pane-move gesture.

use crate::rect::{split_rect, Axis, Rect};
use crate::tabs::{close_tab, remove_pane_node};
use crate::tree::{
    contains_pane, find_node_mut, pane_count, pane_ids, sorted_pane_ids, LayoutTree, Node, PaneId,
    MAX_RATIO, MIN_RATIO,
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
        // Guarded by `contains_pane` before the detach.
        if !attach_next_to(&mut tree.tabs[tab].root, target, pane, zone, ratio) {
            return false;
        }
    }
    tree.tabs[tab].focused = pane;
    true
}

/// Re-splits the `target` leaf inside `root` so `pane` lands on the zone's
/// side at `ratio`. Returns false when `target` is not a leaf of `root`.
fn attach_next_to(
    root: &mut Node,
    target: PaneId,
    pane: PaneId,
    zone: DropZone,
    ratio: f32,
) -> bool {
    let node = match find_node_mut(root, target) {
        Some(node) => node,
        None => return false,
    };
    let old = std::mem::replace(node, Node::Pane { id: pane });
    // Left/Top put the dragged pane in the `first` slot, Right/Bottom in
    // `second` (see `Axis` for the convention).
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
    true
}

/// Move `pane` from tab `from_tab` to land next to `target` in `to_tab`:
/// an edge zone detaches the pane leaf and re-splits the target leaf —
/// the source tab keeps its remaining panes, or closes when the pane was
/// its last one; [`DropZone::Center`] swaps the two panes' slots across
/// the tabs (each pane takes the other's place). The moved pane lands
/// focused and the destination tab becomes active. Pure tree surgery —
/// sessions and pane meta are keyed by pane id and move with it.
/// Returns `false` (tree unchanged) when either tab index is out of
/// range, the tabs are the same (see [`move_pane_to_pane`]), or either
/// pane id is missing from its tab.
pub fn move_pane_across_tabs(
    tree: &mut LayoutTree,
    from_tab: usize,
    pane: PaneId,
    to_tab: usize,
    target: PaneId,
    zone: DropZone,
    ratio: f32,
) -> bool {
    if from_tab == to_tab
        || pane == target
        || from_tab >= tree.tabs.len()
        || to_tab >= tree.tabs.len()
        || !contains_pane(&tree.tabs[from_tab].root, pane)
        || !contains_pane(&tree.tabs[to_tab].root, target)
    {
        return false;
    }
    if zone == DropZone::Center {
        // Pane ids are unique across the whole tree, so rewriting the two
        // leaf ids swaps the panes' slots: each takes the other's place.
        swap_leaf_ids(&mut tree.tabs[from_tab].root, pane, target);
        swap_leaf_ids(&mut tree.tabs[to_tab].root, target, pane);
        // The source tab only re-targets when its focused pane is the one
        // that left (minimal change, close_pane-style); the destination
        // tab always focuses the dragged pane - it arrived at the drop
        // point, mirroring the edge branch.
        if tree.tabs[from_tab].focused == pane {
            tree.tabs[from_tab].focused = target;
        }
        tree.tabs[to_tab].focused = pane;
        tree.active_tab = to_tab;
        return true;
    }
    // Anchor on the destination's content: detaching the pane below may
    // close the source tab and shift tab indices.
    let to_anchor = sorted_pane_ids(&tree.tabs[to_tab].root).first().copied();
    let mut remaining = Vec::new();
    pane_ids(&tree.tabs[from_tab].root, &mut remaining);
    remaining.retain(|&id| id != pane);
    let old_root = std::mem::replace(&mut tree.tabs[from_tab].root, Node::Pane { id: pane });
    match remove_pane_node(old_root, pane) {
        Some(rest) => {
            tree.tabs[from_tab].root = rest;
            if tree.tabs[from_tab].focused == pane {
                // Same focus rule as `close_pane`: the greatest remaining
                // id below the moved pane, else the smallest remaining id.
                let focus = remaining
                    .iter()
                    .copied()
                    .filter(|&id| id < pane)
                    .max()
                    .or_else(|| remaining.iter().copied().min());
                if let Some(id) = focus {
                    tree.tabs[from_tab].focused = id;
                }
            }
        }
        // The pane was the source tab's last one: the placeholder root
        // leaves with the tab itself.
        None => close_tab(tree, from_tab),
    }
    let to_idx =
        match to_anchor.and_then(|id| tree.tabs.iter().position(|t| contains_pane(&t.root, id))) {
            Some(idx) => idx,
            // Defensive: the anchor pane vanished from the tree mid-move.
            None => return false,
        };
    // Guarded by `contains_pane` before the detach.
    if !attach_next_to(&mut tree.tabs[to_idx].root, target, pane, zone, ratio) {
        return false;
    }
    tree.tabs[to_idx].focused = pane;
    tree.active_tab = to_idx;
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
    use crate::tabs::{ensure_next_pane_id, new_tab, new_tree};
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

    /// Two tabs ready for a cross-tab drag: tab0 = v(1 | 2) focused 1 (the
    /// drag source, active), tab1 = Pane{3} focused 3.
    fn two_tabs() -> LayoutTree {
        let mut tree = two_panes();
        new_tab(&mut tree, "b");
        tree.active_tab = 0;
        tree.tabs[0].focused = 1;
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

    #[test]
    fn cross_tab_edge_move_replits_target_and_closes_emptied_source() {
        let mut tree = two_tabs(); // tab0 = v(1 | 2) focused 1, tab1 = Pane{3}
        assert!(move_pane_across_tabs(
            &mut tree,
            0,
            1,
            1,
            3,
            DropZone::Right,
            0.5
        ));
        // The source keeps its remaining pane and focus moves off the
        // departed id (greatest remaining id below it, else the smallest).
        assert_eq!(tree.tabs[0].root, Node::Pane { id: 2 });
        assert_eq!(tree.tabs[0].focused, 2);
        // The destination's leaf 3 is re-split with the dragged pane second.
        assert_eq!(
            tree.tabs[1].root,
            Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(Node::Pane { id: 3 }),
                second: Box::new(Node::Pane { id: 1 }),
            }
        );
        assert_eq!(tree.tabs[1].focused, 1);
        assert_eq!(tree.active_tab, 1);
    }

    #[test]
    fn cross_tab_center_swaps_slots() {
        let mut tree = two_tabs();
        assert!(move_pane_across_tabs(
            &mut tree,
            0,
            1,
            1,
            3,
            DropZone::Center,
            0.9
        ));
        // Each pane takes the other's slot; structure and ratios untouched.
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(Node::Pane { id: 3 }),
                second: Box::new(Node::Pane { id: 2 }),
            }
        );
        assert_eq!(tree.tabs[1].root, Node::Pane { id: 1 });
        assert_eq!(tree.tabs[0].focused, 3);
        assert_eq!(tree.tabs[1].focused, 1);
        assert_eq!(tree.active_tab, 1);
    }

    #[test]
    fn cross_tab_center_focuses_dragged_pane_in_multi_pane_destination() {
        let mut tree = two_tabs();
        // Destination tab gets a second pane and focuses it (not the drop
        // target): the dragged pane still wins focus - it landed at the
        // drop point, mirroring the edge branch.
        assert_eq!(split_pane(&mut tree, 1, 3, Axis::Horizontal), Some(4));
        tree.tabs[1].focused = 4;
        tree.tabs[0].focused = 2;
        assert!(move_pane_across_tabs(
            &mut tree,
            0,
            1,
            1,
            3,
            DropZone::Center,
            0.5
        ));
        assert_eq!(tree.tabs[1].focused, 1);
        // The source tab keeps its focus when the pane that left was not
        // the focused one (minimal change, close_pane-style).
        assert_eq!(tree.tabs[0].focused, 2);
    }

    #[test]
    fn cross_tab_last_pane_closes_source_tab() {
        let mut tree = new_tree("a"); // tab0 = Pane{1}
        ensure_next_pane_id(&mut tree, 3); // id 2 spent elsewhere
        new_tab(&mut tree, "b"); // tab1 = Pane{3}
        assert_eq!(split_pane(&mut tree, 1, 3, Axis::Vertical), Some(4));
        // tab1 = v(3 | 4)
        assert!(move_pane_across_tabs(
            &mut tree,
            0,
            1,
            1,
            3,
            DropZone::Left,
            0.5
        ));
        assert_eq!(tree.tabs.len(), 1);
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(Node::Split {
                    axis: Axis::Vertical,
                    ratio: 0.5,
                    first: Box::new(Node::Pane { id: 1 }),
                    second: Box::new(Node::Pane { id: 3 }),
                }),
                second: Box::new(Node::Pane { id: 4 }),
            }
        );
        assert_eq!(tree.tabs[0].focused, 1);
        assert_eq!(tree.active_tab, 0);
    }

    #[test]
    fn cross_tab_rejects_same_tab_and_missing_panes() {
        let mut tree = two_tabs();
        let before0 = tree.tabs[0].root.clone();
        let before1 = tree.tabs[1].root.clone();
        assert!(!move_pane_across_tabs(
            &mut tree,
            0,
            1,
            0,
            2,
            DropZone::Left,
            0.5
        ));
        assert!(!move_pane_across_tabs(
            &mut tree,
            0,
            99,
            1,
            3,
            DropZone::Left,
            0.5
        ));
        assert!(!move_pane_across_tabs(
            &mut tree,
            0,
            1,
            1,
            99,
            DropZone::Center,
            0.5
        ));
        assert!(!move_pane_across_tabs(
            &mut tree,
            5,
            1,
            1,
            3,
            DropZone::Left,
            0.5
        ));
        assert_eq!(tree.tabs[0].root, before0);
        assert_eq!(tree.tabs[1].root, before1);
        assert_eq!(tree.tabs[0].focused, 1);
        assert_eq!(tree.tabs[1].focused, 3);
        assert_eq!(tree.active_tab, 0);
    }
}
