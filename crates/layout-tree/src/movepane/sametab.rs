//! Same-tab pane moves: detach the dragged leaf and re-split the target.

use super::surgery::{attach_next_to, swap_leaf_ids};
use super::DropZone;
use crate::tabs::remove_pane_node;
use crate::tree::{contains_pane, pane_count, LayoutTree, Node, PaneId};

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
