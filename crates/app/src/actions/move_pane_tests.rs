//! Move tests for the Ctrl+drag pane drop: same-tab re-split, cross-tab
//! migration and the no-op guards around `do_move_pane`.

use super::*;
use crate::state::fresh_state;

/// One tab shaped v(1|2), focus on pane 1.
fn two_pane_state() -> AppState {
    let mut st = fresh_state();
    let tree = &mut st.windows[0].tree;
    tree.tabs.clear();
    tree.tabs.push(layout_tree::Tab {
        title: "work".into(),
        focused: 1,
        root: layout_tree::Node::Split {
            axis: Axis::Vertical,
            ratio: 0.5,
            first: Box::new(layout_tree::Node::Pane { id: 1 }),
            second: Box::new(layout_tree::Node::Pane { id: 2 }),
        },
    });
    tree.active_tab = 0;
    st
}

#[test]
fn right_edge_move_reorders_root_and_focuses_moved_pane() {
    let mut st = two_pane_state();
    st.settings.split_ratio = 0.3;
    st.windows[0].ui.zoom = true;
    let mut dirty = false;
    do_move_pane(&mut st, 0, 1, 2, DropZone::Right, &mut dirty);
    assert_eq!(
        st.windows[0].tree.tabs[0].root,
        layout_tree::Node::Split {
            axis: Axis::Vertical,
            ratio: 0.3,
            first: Box::new(layout_tree::Node::Pane { id: 2 }),
            second: Box::new(layout_tree::Node::Pane { id: 1 }),
        },
        "pane 1 detaches and lands in the right half at the split ratio"
    );
    assert_eq!(st.windows[0].tree.tabs[0].focused, 1);
    assert!(!st.windows[0].ui.zoom, "a move leaves zoomed mode");
    assert!(dirty);
}

#[test]
fn center_swap_exchanges_ids_without_reshaping() {
    let mut st = two_pane_state();
    let mut dirty = false;
    do_move_pane(&mut st, 0, 1, 2, DropZone::Center, &mut dirty);
    assert_eq!(
        st.windows[0].tree.tabs[0].root,
        layout_tree::Node::Split {
            axis: Axis::Vertical,
            ratio: 0.5,
            first: Box::new(layout_tree::Node::Pane { id: 2 }),
            second: Box::new(layout_tree::Node::Pane { id: 1 }),
        },
        "only the leaf ids exchange: axis and ratio untouched"
    );
    assert_eq!(st.windows[0].tree.tabs[0].focused, 1);
    assert!(dirty);
}

#[test]
fn move_onto_itself_is_a_noop() {
    let mut st = two_pane_state();
    let before = st.windows[0].tree.tabs[0].root.clone();
    let mut dirty = false;
    do_move_pane(&mut st, 0, 1, 1, DropZone::Right, &mut dirty);
    assert_eq!(st.windows[0].tree.tabs[0].root, before);
    assert!(!dirty, "rejected move leaves the dirty flag alone");
}

/// Tabs: "a" = v(1|2) focused 1, "b" = Pane{3} focused 3; active 0.
fn two_tab_state() -> AppState {
    let mut st = fresh_state();
    let tree = &mut st.windows[0].tree;
    tree.tabs.clear();
    tree.tabs.push(layout_tree::Tab {
        title: "a".into(),
        focused: 1,
        root: layout_tree::Node::Split {
            axis: Axis::Vertical,
            ratio: 0.5,
            first: Box::new(layout_tree::Node::Pane { id: 1 }),
            second: Box::new(layout_tree::Node::Pane { id: 2 }),
        },
    });
    tree.tabs.push(layout_tree::Tab {
        title: "b".into(),
        focused: 3,
        root: layout_tree::Node::Pane { id: 3 },
    });
    tree.active_tab = 0;
    st
}

#[test]
fn cross_tab_edge_move_lands_in_target_tab() {
    let mut st = two_tab_state();
    let mut dirty = false;
    // Dwell switched the active tab to "b" (index 1): pane 1 drops on
    // pane 3's right edge there.
    do_move_pane(&mut st, 1, 1, 3, DropZone::Right, &mut dirty);
    assert_eq!(
        st.windows[0].tree.tabs[0].root,
        layout_tree::Node::Pane { id: 2 },
        "the source tab keeps its remaining pane"
    );
    assert_eq!(st.windows[0].tree.tabs[0].focused, 2);
    assert_eq!(
        st.windows[0].tree.tabs[1].root,
        layout_tree::Node::Split {
            axis: Axis::Vertical,
            ratio: 0.5,
            first: Box::new(layout_tree::Node::Pane { id: 3 }),
            second: Box::new(layout_tree::Node::Pane { id: 1 }),
        },
        "pane 1 re-splits pane 3's leaf on the right"
    );
    assert_eq!(
        st.windows[0].tree.tabs[1].focused, 1,
        "the moved pane lands focused"
    );
    assert_eq!(
        st.windows[0].tree.active_tab, 1,
        "the destination tab becomes active"
    );
    assert!(!st.windows[0].ui.zoom);
    assert!(dirty);
}

#[test]
fn cross_tab_move_empties_and_closes_source_tab() {
    let mut st = fresh_state();
    let tree = &mut st.windows[0].tree;
    tree.tabs.clear();
    // "a" = lone pane 1, "b" = v(3|2): dragging 1 away empties "a".
    tree.tabs.push(layout_tree::Tab {
        title: "a".into(),
        focused: 1,
        root: layout_tree::Node::Pane { id: 1 },
    });
    tree.tabs.push(layout_tree::Tab {
        title: "b".into(),
        focused: 3,
        root: layout_tree::Node::Split {
            axis: Axis::Vertical,
            ratio: 0.5,
            first: Box::new(layout_tree::Node::Pane { id: 3 }),
            second: Box::new(layout_tree::Node::Pane { id: 2 }),
        },
    });
    tree.active_tab = 0;
    let mut dirty = false;
    do_move_pane(&mut st, 1, 1, 3, DropZone::Left, &mut dirty);
    assert_eq!(
        st.windows[0].tree.tabs.len(),
        1,
        "the emptied source tab closed"
    );
    assert_eq!(
        st.windows[0].tree.tabs[0].root,
        layout_tree::Node::Split {
            axis: Axis::Vertical,
            ratio: 0.5,
            first: Box::new(layout_tree::Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(layout_tree::Node::Pane { id: 1 }),
                second: Box::new(layout_tree::Node::Pane { id: 3 }),
            }),
            second: Box::new(layout_tree::Node::Pane { id: 2 }),
        },
        "pane 1 lands left of pane 3"
    );
    assert_eq!(st.windows[0].tree.tabs[0].focused, 1);
    assert_eq!(st.windows[0].tree.active_tab, 0);
    assert!(dirty);
}

#[test]
fn cross_tab_move_prunes_orphaned_tab_editor() {
    let mut st = two_tab_state();
    // Renaming tab "b" (anchor = its only pane 3): dragging pane 3 away
    // CLOSES the tab, so the anchor pane disappears from the tree and
    // the rename editor is orphaned. (An editor anchored on the MOVED
    // pane would follow it: the pane becomes the lowest id of the
    // destination tab when it lands under higher ids.)
    st.windows[0].ui.tab_edit = Some((3, "renamed".into()));
    let mut dirty = false;
    do_move_pane(&mut st, 0, 3, 1, DropZone::Right, &mut dirty);
    assert_eq!(
        st.windows[0].tree.tabs.len(),
        1,
        "emptied source tab closed"
    );
    assert!(
        st.windows[0].ui.tab_edit.is_none(),
        "orphaned rename editor dropped"
    );
    assert!(dirty);
}

#[test]
fn pane_missing_everywhere_is_a_noop() {
    let mut st = two_tab_state();
    let before: Vec<(String, PaneId, layout_tree::Node)> = st.windows[0]
        .tree
        .tabs
        .iter()
        .map(|t| (t.title.clone(), t.focused, t.root.clone()))
        .collect();
    let mut dirty = false;
    do_move_pane(&mut st, 0, 99, 3, DropZone::Right, &mut dirty);
    let after: Vec<(String, PaneId, layout_tree::Node)> = st.windows[0]
        .tree
        .tabs
        .iter()
        .map(|t| (t.title.clone(), t.focused, t.root.clone()))
        .collect();
    assert_eq!(after, before);
    assert!(!dirty, "rejected move leaves the dirty flag alone");
}
