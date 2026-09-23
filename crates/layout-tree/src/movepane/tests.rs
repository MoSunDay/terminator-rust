use super::*;
use crate::rect::{Axis, Rect};
use crate::tabs::{ensure_next_pane_id, new_tab, new_tree};
use crate::tree::{pane_count, split_pane, LayoutTree, Node, MAX_RATIO, MIN_RATIO};

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
