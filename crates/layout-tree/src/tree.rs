//! The pane tree model: plain data plus free functions (no methods, no
//! traits). Tree/tab lifecycle operations live in [`crate::tabs`], geometry
//! in [`crate::layout`] and focus in [`crate::focus`].

use crate::rect::Axis;
use crate::tabs::alloc_pane_id;

/// Identifier of a terminal pane. Unique within a [`LayoutTree`]; ids start at 1.
pub type PaneId = u64;

/// Lower bound for a split ratio.
pub const MIN_RATIO: f32 = 0.05;
/// Upper bound for a split ratio.
pub const MAX_RATIO: f32 = 0.95;

/// A node of a tab's pane tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// A leaf: a single terminal pane.
    Pane {
        /// The pane's unique id.
        id: PaneId,
    },
    /// An inner node dividing its area between two subtrees with a divider.
    Split {
        /// Orientation of the divider (see [`crate::rect::Axis`]).
        axis: Axis,
        /// Fraction of the usable length given to `first`,
        /// always in `MIN_RATIO..=MAX_RATIO`.
        ratio: f32,
        /// First (top / left) child.
        first: Box<Node>,
        /// Second (bottom / right) child.
        second: Box<Node>,
    },
}

/// A tab: a title plus its pane tree and the focused pane.
#[derive(Debug, Clone)]
pub struct Tab {
    /// User-visible tab title.
    pub title: String,
    /// Root of the binary pane tree.
    pub root: Node,
    /// Pane currently focused in this tab.
    pub focused: PaneId,
}

/// Whole-layout state: all tabs, the active tab and the pane id allocator.
#[derive(Debug, Clone)]
pub struct LayoutTree {
    /// All tabs, in display order.
    pub tabs: Vec<Tab>,
    /// Index of the active tab (kept valid by the tab operations).
    pub active_tab: usize,
    /// Next pane id handed out by [`crate::tabs::alloc_pane_id`].
    pub(crate) next_pane_id: PaneId,
}

/// Number of panes (leaves) in `node`.
pub fn pane_count(node: &Node) -> usize {
    match node {
        Node::Pane { .. } => 1,
        Node::Split { first, second, .. } => pane_count(first) + pane_count(second),
    }
}

/// Collects the pane ids of `node` in tree order (first subtree first).
pub fn pane_ids(node: &Node, out: &mut Vec<PaneId>) {
    match node {
        Node::Pane { id } => out.push(*id),
        Node::Split { first, second, .. } => {
            pane_ids(first, out);
            pane_ids(second, out);
        }
    }
}

/// The pane ids of `node`, sorted ascending.
pub fn sorted_pane_ids(node: &Node) -> Vec<PaneId> {
    let mut ids = Vec::new();
    pane_ids(node, &mut ids);
    ids.sort_unstable();
    ids
}

/// First node in tree order whose pane id is `pane`.
pub fn find_node<'a>(node: &'a Node, pane: PaneId) -> Option<&'a Node> {
    match node {
        Node::Pane { id } if *id == pane => Some(node),
        Node::Pane { .. } => None,
        Node::Split { first, second, .. } => {
            find_node(first, pane).or_else(|| find_node(second, pane))
        }
    }
}

/// Mutable variant of [`find_node`].
pub fn find_node_mut<'a>(node: &'a mut Node, pane: PaneId) -> Option<&'a mut Node> {
    match node {
        Node::Pane { id } if *id == pane => Some(node),
        Node::Pane { .. } => None,
        Node::Split { first, second, .. } => {
            find_node_mut(first, pane).or_else(|| find_node_mut(second, pane))
        }
    }
}

/// True when `node` contains a pane with id `pane`.
pub fn contains_pane(node: &Node, pane: PaneId) -> bool {
    find_node(node, pane).is_some()
}

/// Replaces the pane `pane` of tab `tab` with `Split { axis, 0.5, old, new }`
/// and focuses the new pane. Returns its id, or `None` (unchanged tree, no id
/// consumed) when the tab or pane does not exist.
pub fn split_pane(
    tree: &mut LayoutTree,
    tab: usize,
    pane: PaneId,
    axis: Axis,
) -> Option<PaneId> {
    if !contains_pane(&tree.tabs.get(tab)?.root, pane) {
        return None;
    }
    let new_id = alloc_pane_id(tree);
    let t = tree.tabs.get_mut(tab)?;
    let target = find_node_mut(&mut t.root, pane)?;
    let old = std::mem::replace(target, Node::Pane { id: new_id });
    *target = Node::Split {
        axis,
        ratio: 0.5,
        first: Box::new(old),
        second: Box::new(Node::Pane { id: new_id }),
    };
    t.focused = new_id;
    Some(new_id)
}

/// Axis of the innermost split containing `pane` (its direct parent).
/// `None` when `pane` is the root pane or absent from the tree.
pub fn parent_axis(node: &Node, pane: PaneId) -> Option<Axis> {
    match node {
        Node::Pane { .. } => None,
        Node::Split { axis, first, second, .. } => {
            if contains_pane(first, pane) {
                parent_axis(first, pane).or(Some(*axis))
            } else if contains_pane(second, pane) {
                parent_axis(second, pane).or(Some(*axis))
            } else {
                None
            }
        }
    }
}

/// Current ratio of the innermost split containing `pane` (see
/// [`parent_axis`] for the `None` cases).
pub fn parent_ratio(node: &Node, pane: PaneId) -> Option<f32> {
    match node {
        Node::Pane { .. } => None,
        Node::Split { ratio, first, second, .. } => {
            if contains_pane(first, pane) {
                parent_ratio(first, pane).or(Some(*ratio))
            } else if contains_pane(second, pane) {
                parent_ratio(second, pane).or(Some(*ratio))
            } else {
                None
            }
        }
    }
}

/// Sets the ratio of the innermost split containing `pane` (its direct
/// parent), clamped to `MIN_RATIO..=MAX_RATIO`. Returns whether a parent
/// split was found and updated.
pub fn set_parent_ratio(node: &mut Node, pane: PaneId, ratio: f32) -> bool {
    let clamped = ratio.clamp(MIN_RATIO, MAX_RATIO);
    match node {
        Node::Pane { .. } => false,
        Node::Split { ratio: node_ratio, first, second, .. } => {
            let child: &mut Node = if contains_pane(first, pane) {
                first
            } else if contains_pane(second, pane) {
                second
            } else {
                return false;
            };
            if set_parent_ratio(child, pane, clamped) {
                return true;
            }
            *node_ratio = clamped;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::{new_tree, next_pane_id};

    fn pane(id: PaneId) -> Node {
        Node::Pane { id }
    }

    /// Root split horizontally (pane 1 top, pane 2 bottom), then pane 1
    /// split vertically (pane 1 left, pane 3 right).
    fn three_panes() -> LayoutTree {
        let mut tree = new_tree("main");
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Horizontal), Some(2));
        assert_eq!(split_pane(&mut tree, 0, 1, Axis::Vertical), Some(3));
        tree
    }

    #[test]
    fn split_builds_expected_structure() {
        let tree = three_panes();
        let expected = Node::Split {
            axis: Axis::Horizontal,
            ratio: 0.5,
            first: Box::new(Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(pane(1)),
                second: Box::new(pane(3)),
            }),
            second: Box::new(pane(2)),
        };
        assert_eq!(tree.tabs[0].root, expected);
        assert_eq!(tree.tabs[0].focused, 3); // newest pane is focused
        assert_eq!(next_pane_id(&tree), 4);
    }

    #[test]
    fn pane_count_and_ids() {
        let tree = three_panes();
        let root = &tree.tabs[0].root;
        assert_eq!(pane_count(root), 3);
        let mut ids = Vec::new();
        pane_ids(root, &mut ids);
        assert_eq!(ids, vec![1, 3, 2]); // tree order
        assert_eq!(sorted_pane_ids(root), vec![1, 2, 3]);
    }

    #[test]
    fn find_and_contains() {
        let tree = three_panes();
        let root = &tree.tabs[0].root;
        assert_eq!(find_node(root, 2), Some(&pane(2)));
        assert_eq!(find_node(root, 3), Some(&pane(3)));
        assert_eq!(find_node(root, 99), None);
        assert!(contains_pane(root, 1));
        assert!(!contains_pane(root, 42));
        let mut root = tree.tabs[0].root.clone();
        let found = find_node_mut(&mut root, 1);
        assert!(matches!(found, Some(Node::Pane { id: 1 })));
        assert!(find_node_mut(&mut root, 9).is_none());
    }

    #[test]
    fn split_missing_pane_consumes_no_id() {
        let mut tree = three_panes();
        assert_eq!(split_pane(&mut tree, 0, 99, Axis::Horizontal), None);
        assert_eq!(split_pane(&mut tree, 7, 1, Axis::Horizontal), None);
        assert_eq!(next_pane_id(&tree), 4); // nothing allocated
        assert_eq!(pane_count(&tree.tabs[0].root), 3);
    }

    #[test]
    fn ratio_targets_innermost_parent() {
        let mut tree = three_panes();
        let root = &mut tree.tabs[0].root;
        assert!(set_parent_ratio(root, 1, 0.7));
        assert_eq!(parent_ratio(root, 1), Some(0.7)); // vertical split changed
        assert_eq!(parent_axis(root, 1), Some(Axis::Vertical));
        assert_eq!(parent_ratio(root, 2), Some(0.5)); // untouched
        assert_eq!(parent_axis(root, 2), Some(Axis::Horizontal));
    }

    #[test]
    fn ratio_is_clamped() {
        let mut tree = three_panes();
        let root = &mut tree.tabs[0].root;
        assert!(set_parent_ratio(root, 1, 0.0));
        assert_eq!(parent_ratio(root, 1), Some(MIN_RATIO));
        assert!(set_parent_ratio(root, 1, 2.0));
        assert_eq!(parent_ratio(root, 1), Some(MAX_RATIO));
    }

    #[test]
    fn ratio_of_missing_or_root_pane_fails() {
        let mut tree = three_panes();
        assert!(!set_parent_ratio(&mut tree.tabs[0].root, 99, 0.5));
        let mut single = new_tree("solo");
        assert_eq!(parent_axis(&single.tabs[0].root, 1), None);
        assert_eq!(parent_ratio(&single.tabs[0].root, 1), None);
        assert!(!set_parent_ratio(&mut single.tabs[0].root, 1, 0.5));
    }
}
