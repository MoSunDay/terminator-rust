//! Leaf-level tree surgery shared by the same-tab and cross-tab movers.

use super::DropZone;
use crate::rect::Axis;
use crate::tree::{find_node_mut, Node, PaneId, MAX_RATIO, MIN_RATIO};

/// Re-splits the `target` leaf inside `root` so `pane` lands on the zone's
/// side at `ratio`. Returns false when `target` is not a leaf of `root`.
pub(super) fn attach_next_to(
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

/// Rewrites leaf id `a` to `b` and vice versa throughout `node`, leaving
/// the structure and split ratios untouched.
pub(super) fn swap_leaf_ids(node: &mut Node, a: PaneId, b: PaneId) {
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
