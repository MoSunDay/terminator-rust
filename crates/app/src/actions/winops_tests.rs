//! Cross-window tab surgery tests: real `AppState` windows via
//! `windows::spawn`, asserting pane survival + focus containment.
use super::*;
use crate::session_map::session_map;
use crate::state::{fresh_state, tab_anchor, ui_state};
use crate::windows;

fn anchors(st: &AppState, wi: usize) -> Vec<PaneId> {
    st.windows[wi].tree.tabs.iter().map(tab_anchor).collect()
}

#[test]
fn move_tab_to_window_moves_tab_and_keeps_panes() {
    let mut st = fresh_state();
    let mut sess = session_map();
    let mut dirty = false;
    windows::spawn(&mut st, &mut sess, &mut dirty);
    let anchor = tab_anchor(&st.windows[1].tree.tabs[0]);
    let panes_before: Vec<PaneId> = st.panes.keys().copied().collect();
    let root_id = st.windows[0].id;
    assert!(do_move_tab_to_window(&mut st, &mut dirty, anchor, root_id));
    assert_eq!(st.windows.len(), 1, "emptied secondary removes itself");
    assert_eq!(anchors(&st, 0), vec![1, anchor], "appended after root tabs");
    assert_eq!(
        st.panes.keys().copied().collect::<Vec<_>>(),
        panes_before,
        "sessions/meta stay (global by pane id)"
    );
    assert_eq!(
        tab_anchor(&st.windows[0].tree.tabs[st.windows[0].tree.active_tab]),
        anchor,
        "moved tab is active in the destination"
    );
    assert_eq!(st.focus, 0);
    assert!(dirty);
    // Future allocations in the root must not collide with the moved
    // pane ids: the destination allocator was seeded from the budget.
    let id = layout_tree::alloc_pane_id(&mut st.windows[0].tree);
    assert!(id > anchor, "alloc_pane_id stays above the moved ids");
}

#[test]
fn move_tab_out_of_root_leaves_root_window_alive() {
    let mut st = fresh_state();
    let mut sess = session_map();
    let mut dirty = false;
    windows::spawn(&mut st, &mut sess, &mut dirty);
    let anchor = tab_anchor(&st.windows[0].tree.tabs[0]);
    let dst = st.windows[1].id;
    // The receiving window keeps its own (seeded) tab: the moved tab
    // lands after it, like a second Ctrl+Shift+N window.
    let seed = tab_anchor(&st.windows[1].tree.tabs[0]);
    assert!(do_move_tab_to_window(&mut st, &mut dirty, anchor, dst));
    assert_eq!(st.windows.len(), 2, "the root keeps its viewport");
    assert!(
        st.windows[0].tree.tabs.is_empty(),
        "screen()'s respawn branch owns the empty root"
    );
    assert_eq!(anchors(&st, 1), vec![seed, anchor]);
    assert!(st.panes.contains_key(&anchor), "pane meta survives");
    assert_eq!(st.focus, 1);
}

#[test]
fn merge_windows_folds_all_windows_into_root_in_order() {
    let mut st = fresh_state();
    let mut sess = session_map();
    let mut dirty = false;
    // Root gets a second tab (active) so "same active tab" is a real
    // assertion, not active_tab == 0 by accident.
    st.active = 0;
    crate::actions::do_new_tab(&mut st, &mut sess, remote::PaneKind::Local, &mut dirty);
    assert_eq!(st.windows[0].tree.active_tab, 1);
    let keep = tab_anchor(&st.windows[0].tree.tabs[1]);
    windows::spawn(&mut st, &mut sess, &mut dirty);
    windows::spawn(&mut st, &mut sess, &mut dirty);
    let w2 = tab_anchor(&st.windows[1].tree.tabs[0]);
    let w3 = tab_anchor(&st.windows[2].tree.tabs[0]);
    let panes_before: Vec<PaneId> = st.panes.keys().copied().collect();
    do_merge_windows(&mut st, &mut dirty);
    assert_eq!(st.windows.len(), 1);
    assert_eq!(
        anchors(&st, 0),
        vec![1, keep, w2, w3],
        "root first, then window order"
    );
    assert_eq!(
        tab_anchor(&st.windows[0].tree.tabs[st.windows[0].tree.active_tab]),
        keep,
        "root's active tab is the SAME tab"
    );
    assert_eq!(
        st.panes.keys().copied().collect::<Vec<_>>(),
        panes_before,
        "nothing terminated"
    );
    assert_eq!(st.focus, 0);
    assert!(dirty);
}

#[test]
fn merge_windows_is_a_no_op_with_one_window() {
    let mut st = fresh_state();
    let mut dirty = false;
    do_merge_windows(&mut st, &mut dirty);
    assert_eq!(st.windows.len(), 1);
    assert!(!dirty);
}

#[test]
fn strip_hit_excludes_source_and_prefers_last() {
    let mut sc = WinScreens::default();
    sc.strip.insert(
        1,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 30.0)),
    );
    sc.strip.insert(
        2,
        egui::Rect::from_min_max(egui::pos2(200.0, 0.0), egui::pos2(300.0, 30.0)),
    );
    sc.strip.insert(
        3,
        egui::Rect::from_min_max(egui::pos2(50.0, 0.0), egui::pos2(150.0, 30.0)),
    );
    let at = |x: f32, y: f32| egui::pos2(x, y);
    // 1 and 3 overlap at x=60..100: the later window wins.
    assert_eq!(strip_hit(&sc, 1, at(60.0, 10.0), &[1, 3]), Some(3));
    assert_eq!(strip_hit(&sc, 3, at(60.0, 10.0), &[1, 3]), Some(1));
    assert_eq!(strip_hit(&sc, 2, at(60.0, 10.0), &[1, 2, 3]), Some(3));
    assert_eq!(strip_hit(&sc, 1, at(250.0, 10.0), &[1, 2, 3]), Some(2));
    assert_eq!(strip_hit(&sc, 1, at(180.0, 10.0), &[1, 2, 3]), None);
    assert_eq!(strip_hit(&sc, 1, at(60.0, 40.0), &[1, 3]), None);
    // Stale published rects never match once the id left the order.
    assert_eq!(strip_hit(&sc, 2, at(60.0, 10.0), &[2, 3]), Some(3));
}

#[test]
fn move_tab_next_window_detaches_into_fresh_window() {
    let mut st = fresh_state();
    let mut sess = session_map();
    let mut ui = ui_state();
    let mut dirty = false;
    let anchor = tab_anchor(&st.windows[0].tree.tabs[0]);
    do_move_tab_next_window(&mut st, &mut sess, &mut ui, &mut dirty);
    assert_eq!(st.windows.len(), 2, "a fresh window was spawned");
    assert_eq!(anchors(&st, 1), vec![anchor], "exactly the moved tab");
    assert!(
        st.windows[0].tree.tabs.is_empty(),
        "root emptied; screen() respawns a tab"
    );
    // The seed shell pane closed through the regular path: meta gone,
    // session terminated. (fresh_state holds pane 1; windows::spawn
    // draws the next id from the global budget -> seed pane is 2.)
    assert!(!st.panes.contains_key(&2));
    assert!(!sess.map.contains_key(&2));
    assert_eq!(
        st.panes.keys().copied().collect::<Vec<_>>(),
        vec![anchor],
        "only the moved pane keeps its meta"
    );
    assert!(st.panes.contains_key(&anchor));
    assert_eq!(st.focus, 1, "the new window is focused");
}

#[test]
fn move_tab_next_window_cycles_to_next_window() {
    let mut st = fresh_state();
    let mut sess = session_map();
    let mut ui = ui_state();
    let mut dirty = false;
    windows::spawn(&mut st, &mut sess, &mut dirty);
    let root_anchor = tab_anchor(&st.windows[0].tree.tabs[0]);
    let second = tab_anchor(&st.windows[1].tree.tabs[0]);
    // Focus sits on the spawned window: next (cyclic) is the root.
    do_move_tab_next_window(&mut st, &mut sess, &mut ui, &mut dirty);
    assert_eq!(anchors(&st, 0), vec![root_anchor, second]);
    assert_eq!(st.windows.len(), 1, "emptied secondary removed itself");
    assert_eq!(
        tab_anchor(&st.windows[0].tree.tabs[st.windows[0].tree.active_tab]),
        second
    );
    assert_eq!(st.focus, 0);
}
