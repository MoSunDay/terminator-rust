//! Lifecycle operations on a [`LayoutTree`]: creating trees and tabs,
//! allocating pane ids and closing panes/tabs.

use crate::tree::{contains_pane, pane_ids, LayoutTree, Node, PaneId, Tab};

/// Creates a fresh layout with a single tab holding a single pane (id 1,
/// focused) and makes it the active tab.
pub fn new_tree(title: &str) -> LayoutTree {
    LayoutTree {
        tabs: vec![Tab {
            title: title.to_string(),
            root: Node::Pane { id: 1 },
            focused: 1,
        }],
        active_tab: 0,
        next_pane_id: 2,
    }
}

/// The id the next allocated pane will receive (see [`alloc_pane_id`]).
pub fn next_pane_id(tree: &LayoutTree) -> PaneId {
    tree.next_pane_id
}

/// Allocates a fresh, unique pane id.
pub fn alloc_pane_id(tree: &mut LayoutTree) -> PaneId {
    let id = tree.next_pane_id;
    tree.next_pane_id += 1;
    id
}

/// Appends a new tab with a single focused pane and makes it active.
/// Returns the new tab's index.
pub fn new_tab(tree: &mut LayoutTree, title: &str) -> usize {
    let id = alloc_pane_id(tree);
    tree.tabs.push(Tab {
        title: title.to_string(),
        root: Node::Pane { id },
        focused: id,
    });
    tree.active_tab = tree.tabs.len() - 1;
    tree.active_tab
}

/// Removes tab `tab` (no-op when out of range) and keeps `active_tab` valid:
/// closing a tab before the active one shifts the index down, and the index
/// is clamped when the active tab itself disappears.
pub fn close_tab(tree: &mut LayoutTree, tab: usize) {
    if tab >= tree.tabs.len() {
        return;
    }
    tree.tabs.remove(tab);
    if tree.active_tab > tab {
        tree.active_tab -= 1;
    }
    if tree.active_tab >= tree.tabs.len() {
        tree.active_tab = tree.tabs.len().saturating_sub(1);
    }
}

/// Removes `pane` from tab `tab`. A parent split left with a single child is
/// replaced by that child. Focus moves to the nearest remaining pane: the
/// predecessor in pane id order, or the smallest remaining id when the closed
/// pane had none. If the closed pane was the tab's last one, the whole tab is
/// removed (see [`close_tab`]).
///
/// Returns `Some(())` on success, `None` (tree unchanged) when the tab index
/// or pane id is invalid.
pub fn close_pane(tree: &mut LayoutTree, tab: usize, pane: PaneId) -> Option<()> {
    let t = tree.tabs.get(tab)?;
    if !contains_pane(&t.root, pane) {
        return None;
    }
    let mut remaining = Vec::new();
    pane_ids(&t.root, &mut remaining);
    remaining.retain(|&id| id != pane);
    // Placeholder; overwritten by the match below in either branch.
    let old_root = std::mem::replace(&mut tree.tabs[tab].root, Node::Pane { id: pane });
    match remove_pane_node(old_root, pane) {
        Some(new_root) => {
            tree.tabs[tab].root = new_root;
            let focus = remaining
                .iter()
                .copied()
                .filter(|&id| id < pane)
                .max()
                .or_else(|| remaining.iter().copied().min());
            if let Some(id) = focus {
                tree.tabs[tab].focused = id;
            }
        }
        None => close_tab(tree, tab),
    }
    Some(())
}

/// Removes `pane` from `node` by value, replacing a split that lost a child
/// with the surviving child. `None` means the subtree is now empty.
fn remove_pane_node(node: Node, pane: PaneId) -> Option<Node> {
    match node {
        Node::Pane { id } => (id != pane).then_some(node),
        Node::Split { axis, ratio, first, second } => {
            if contains_pane(&first, pane) {
                match remove_pane_node(*first, pane) {
                    Some(f) => Some(Node::Split { axis, ratio, first: Box::new(f), second }),
                    None => Some(*second),
                }
            } else if contains_pane(&second, pane) {
                match remove_pane_node(*second, pane) {
                    Some(s) => Some(Node::Split { axis, ratio, first, second: Box::new(s) }),
                    None => Some(*first),
                }
            } else {
                Some(Node::Split { axis, ratio, first, second })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rect::Axis;
    use crate::tree::{pane_count, pane_ids, split_pane};

    /// 3 panes: root horizontal (1 top | 2 bottom), pane 1 split vertically
    /// into 1 | 3 (so pane 3 sits top-right).
    fn three_panes() -> LayoutTree {
        let mut tree = new_tree("main");
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Horizontal), Some(2));
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Vertical), Some(3));
        tree
    }

    #[test]
    fn new_tree_has_one_focused_pane() {
        let tree = new_tree("main");
        assert_eq!(tree.tabs.len(), 1);
        assert_eq!(tree.active_tab, 0);
        assert_eq!(tree.tabs[0].title, "main");
        assert_eq!(tree.tabs[0].root, Node::Pane { id: 1 });
        assert_eq!(tree.tabs[0].focused, 1);
        assert_eq!(next_pane_id(&tree), 2);
    }

    #[test]
    fn alloc_ids_are_unique_and_increasing() {
        let mut tree = new_tree("t");
        assert_eq!(alloc_pane_id(&mut tree), 2);
        assert_eq!(alloc_pane_id(&mut tree), 3);
        assert_eq!(next_pane_id(&tree), 4);
    }

    #[test]
    fn new_tab_appends_and_activates() {
        let mut tree = three_panes(); // next id: 4
        assert_eq!(new_tab(&mut tree, "second"), 1);
        assert_eq!(tree.tabs.len(), 2);
        assert_eq!(tree.active_tab, 1);
        assert_eq!(tree.tabs[1].title, "second");
        assert_eq!(tree.tabs[1].root, Node::Pane { id: 4 });
        assert_eq!(tree.tabs[1].focused, 4);
        assert_eq!(new_tab(&mut tree, "third"), 2);
        assert_eq!(tree.active_tab, 2);
    }

    #[test]
    fn close_tab_before_active_shifts_index() {
        let mut tree = three_panes();
        new_tab(&mut tree, "second"); // active = 1
        close_tab(&mut tree, 0);
        assert_eq!(tree.tabs.len(), 1);
        assert_eq!(tree.active_tab, 0);
        assert_eq!(tree.tabs[0].title, "second");
    }

    #[test]
    fn close_active_tab_clamps() {
        let mut tree = three_panes();
        new_tab(&mut tree, "second"); // active = 1
        close_tab(&mut tree, 1);
        assert_eq!(tree.active_tab, 0);
        close_tab(&mut tree, 0); // last tab
        assert!(tree.tabs.is_empty());
        assert_eq!(tree.active_tab, 0);
        close_tab(&mut tree, 0); // out of range: no-op
        assert!(tree.tabs.is_empty());
    }

    #[test]
    fn close_pane_collapses_split_and_refocuses_predecessor() {
        let mut tree = three_panes(); // panes 1,2,3 focused on 3
        assert_eq!(close_pane(&mut tree, 0, 3), Some(()));
        assert_eq!(pane_count(&tree.tabs[0].root), 2);
        let mut ids = Vec::new();
        pane_ids(&tree.tabs[0].root, &mut ids);
        assert_eq!(ids, vec![1, 2]);
        // Vertical split collapsed: root is now 1 | 2 horizontally.
        assert_eq!(
            tree.tabs[0].root,
            Node::Split {
                axis: Axis::Horizontal,
                ratio: 0.5,
                first: Box::new(Node::Pane { id: 1 }),
                second: Box::new(Node::Pane { id: 2 })
            }
        );
        assert_eq!(tree.tabs[0].focused, 2); // predecessor of 3
    }

    #[test]
    fn close_pane_falls_back_to_smallest_remaining() {
        let mut tree = three_panes(); // panes 1,2,3
        assert_eq!(close_pane(&mut tree, 0, 1), Some(())); // no predecessor of 1
        let mut ids = Vec::new();
        pane_ids(&tree.tabs[0].root, &mut ids);
        assert_eq!(ids, vec![3, 2]); // pane 3 took pane 1's place
        assert_eq!(tree.tabs[0].focused, 2); // smallest remaining id
        assert_eq!(pane_count(&tree.tabs[0].root), 2);
    }

    #[test]
    fn closing_down_to_one_pane_yields_bare_pane() {
        let mut tree = three_panes();
        close_pane(&mut tree, 0, 3);
        close_pane(&mut tree, 0, 1);
        assert_eq!(tree.tabs[0].root, Node::Pane { id: 2 });
        assert_eq!(tree.tabs[0].focused, 2);
    }

    #[test]
    fn closing_last_pane_removes_tab() {
        let mut tree = three_panes();
        new_tab(&mut tree, "second"); // active = 1, pane 4
        close_pane(&mut tree, 1, 4); // tab 1 had a single pane
        assert_eq!(tree.tabs.len(), 1);
        assert_eq!(tree.active_tab, 0); // clamped after its tab vanished
        close_pane(&mut tree, 0, 1);
        close_pane(&mut tree, 0, 2);
        close_pane(&mut tree, 0, 3); // last pane of the last tab
        assert!(tree.tabs.is_empty());
        assert_eq!(tree.active_tab, 0);
    }

    #[test]
    fn close_pane_invalid_targets_are_noops() {
        let mut tree = three_panes();
        assert_eq!(close_pane(&mut tree, 0, 99), None);
        assert_eq!(close_pane(&mut tree, 5, 1), None);
        assert_eq!(pane_count(&tree.tabs[0].root), 3);
        assert_eq!(tree.tabs[0].focused, 3);
    }
}
