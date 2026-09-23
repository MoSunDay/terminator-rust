//! Cross-tab pane migration: move a pane leaf between two tabs of the tree.

use super::surgery::{attach_next_to, swap_leaf_ids};
use super::DropZone;
use crate::tabs::{close_tab, remove_pane_node};
use crate::tree::{contains_pane, pane_ids, sorted_pane_ids, LayoutTree, Node, PaneId};

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
