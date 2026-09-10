//! State transitions that coordinate the layout tree, pane metadata and
//! live sessions (spawn/close/respawn). Pure tree edits live in state.rs.

use std::time::Instant;

use layout_tree::{close_pane, close_tab, new_tab, Axis, PaneId};
use log::warn;
use remote::{PaneKind, EXIT_NO_ZELLIJ};

use crate::session_map::{self, SessionMap};
use crate::state::{
    self, new_pane_meta, Action, AppState, PaneAction, UiState, WindowState, WindowUi,
};

/// Spawn sessions for every pane that lacks one; drop orphaned sessions.
pub fn ensure_sessions(st: &AppState, sess: &mut SessionMap) {
    let ids = state::all_pane_ids(st);
    let stale: Vec<PaneId> = sess
        .map
        .keys()
        .filter(|id| !ids.contains(id))
        .copied()
        .collect();
    for id in stale {
        session_map::terminate(sess, id);
    }
    let now = Instant::now();
    for id in ids {
        if sess.map.contains_key(&id) {
            continue;
        }
        if session_map::spawn_blocked(sess, id, now) {
            continue;
        }
        spawn_pane(st, sess, id);
    }
}

pub(crate) fn spawn_pane(st: &AppState, sess: &mut SessionMap, id: PaneId) {
    if let Some(meta) = st.panes.get(&id) {
        match session_map::spawn_meta(
            meta,
            &st.theme_name,
            session_map::START_COLS,
            session_map::START_ROWS,
        ) {
            Ok(s) => {
                sess.map.insert(id, s);
                sess.retry_at.remove(&id);
            }
            Err(e) => {
                warn!("spawn pane {id}: {e}");
                sess.retry_at
                    .insert(id, Instant::now() + session_map::SPAWN_BACKOFF);
            }
        }
    }
}

pub fn do_new_tab(
    st: &mut AppState,
    sess: &mut SessionMap,
    kind: PaneKind,
    dirty: &mut bool,
) -> usize {
    let title = match &kind {
        PaneKind::Local => "shell".to_string(),
        PaneKind::Remote(t) => {
            if t.label.is_empty() {
                t.host.clone()
            } else {
                t.label.clone()
            }
        }
    };
    let wi = st.active_idx();
    st.seed_alloc(wi);
    let idx = match st.windows.get_mut(wi) {
        Some(w) => new_tab(&mut w.tree, &title),
        None => return 0,
    };
    st.collect_alloc();
    let pane = st
        .windows
        .get(wi)
        .and_then(|w| w.tree.tabs.get(idx).map(|t| t.focused))
        .unwrap_or(0);
    st.panes.insert(pane, new_pane_meta(kind));
    spawn_pane(st, sess, pane);
    *dirty = true;
    idx
}

/// Split `pane` (or the focused pane when `pane` is `None`), spawning a local
/// shell in the new half. Returns the new pane id.
pub fn do_split(
    st: &mut AppState,
    sess: &mut SessionMap,
    tab: usize,
    pane: Option<PaneId>,
    axis: Axis,
    dirty: &mut bool,
) -> Option<PaneId> {
    let target = pane.or_else(|| {
        st.win()
            .and_then(|w| w.tree.tabs.get(tab).map(|t| t.focused))
    })?;
    let new_id = state::split_tree_pane(st, tab, target, axis)?;
    spawn_pane(st, sess, new_id);
    *dirty = true;
    Some(new_id)
}

pub fn do_close_pane(
    st: &mut AppState,
    sess: &mut SessionMap,
    ui: &mut UiState,
    tab: usize,
    pane: PaneId,
    dirty: &mut bool,
) {
    let wi = st.active_idx();
    let state::AppState { panes, windows, .. } = st;
    if let Some(w) = windows.get_mut(wi) {
        let state::WindowState { tree, ui: wui, .. } = w;
        if close_pane(tree, tab, pane).is_some() {
            panes.remove(&pane);
            session_map::terminate(sess, pane);
            wui.zoom = false;
            if wui.pane_edit.as_ref().is_some_and(|(p, _)| *p == pane) {
                wui.pane_edit = None;
            }
            if wui.color_open == Some(pane) {
                wui.color_open = None;
            }
            if wui.trans_open == Some(pane) {
                wui.trans_open = None;
            }
            *dirty = true;
        }
        prune_tab_edit(tree, wui);
    }
    handle_empty_window(st, ui, dirty);
}

/// Drop a tab-rename edit whose anchor pane no longer exists.
fn prune_tab_edit(tree: &layout_tree::LayoutTree, wui: &mut WindowUi) {
    if wui
        .tab_edit
        .as_ref()
        .is_some_and(|(anchor, _)| state::tab_edit_orphaned(tree, *anchor))
    {
        wui.tab_edit = None;
    }
}

pub fn do_close_tab(
    st: &mut AppState,
    sess: &mut SessionMap,
    ui: &mut UiState,
    tab: usize,
    dirty: &mut bool,
) {
    let wi = st.active_idx();
    let AppState { panes, windows, .. } = st;
    if let Some(w) = windows.get_mut(wi) {
        let WindowState { tree, ui: wui, .. } = w;
        if let Some(t) = tree.tabs.get(tab) {
            let panes_to_close = layout_tree::sorted_pane_ids(&t.root);
            for pane in panes_to_close {
                panes.remove(&pane);
                session_map::terminate(sess, pane);
                if wui.pane_edit.as_ref().is_some_and(|(p, _)| *p == pane) {
                    wui.pane_edit = None;
                }
                if wui.color_open == Some(pane) {
                    wui.color_open = None;
                }
                if wui.trans_open == Some(pane) {
                    wui.trans_open = None;
                }
            }
        }
        close_tab(tree, tab);
        wui.zoom = false;
        prune_tab_edit(tree, wui);
    }
    handle_empty_window(st, ui, dirty);
    *dirty = true;
}

/// What an empty ACTIVE tree means: the root respawns a fresh tab (or quits
/// when it is the only window left); a secondary window removes itself so
/// its OS window goes away. The caller already closed/terminated all panes
/// of the closed tab/pane, so only the bookkeeping is left.
fn handle_empty_window(st: &mut AppState, ui: &mut UiState, dirty: &mut bool) {
    let wi = st.active_idx();
    if !st.windows.get(wi).is_some_and(|w| w.tree.tabs.is_empty()) {
        return;
    }
    if wi == 0 {
        // Root never dies while siblings exist; screen() respawns a tab.
        if st.windows.len() == 1 {
            ui.quitting = true;
        }
        return;
    }
    st.windows.remove(wi);
    st.retarget_after_remove(wi);
    *dirty = true;
}

/// Kill and respawn a pane's process with the same plan. Remotes go through
/// the idempotent bootstrap again (degraded flag reset).
pub fn do_respawn(st: &mut AppState, sess: &mut SessionMap, pane: PaneId, dirty: &mut bool) {
    let Some(meta) = st.panes.get_mut(&pane) else {
        return;
    };
    meta.degraded = false;
    session_map::terminate(sess, pane);
    if let Some(meta) = st.panes.get(&pane) {
        match session_map::spawn_meta(
            meta,
            &st.theme_name,
            session_map::START_COLS,
            session_map::START_ROWS,
        ) {
            Ok(s) => {
                sess.map.insert(pane, s);
                sess.retry_at.remove(&pane);
            }
            Err(e) => {
                warn!("respawn pane {pane}: {e}");
                sess.retry_at
                    .insert(pane, Instant::now() + session_map::SPAWN_BACKOFF);
            }
        }
    }
    *dirty = true;
}

/// Auto-degrade remote panes that exited with the "no zellij" marker.
pub fn auto_degrade(st: &mut AppState, sess: &mut SessionMap, dirty: &mut bool) {
    let ids: Vec<PaneId> = sess
        .map
        .iter()
        .filter(|(_, s)| s.exit == Some(EXIT_NO_ZELLIJ))
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        if !matches!(
            st.panes.get(&id).map(|m| &m.kind),
            Some(PaneKind::Remote(_))
        ) {
            continue;
        }
        if st.panes.get(&id).map(|m| m.degraded).unwrap_or(true) {
            continue;
        }
        if let Some(m) = st.panes.get_mut(&id) {
            m.degraded = true;
        }
        session_map::terminate(sess, id);
        if let Some(m) = st.panes.get(&id) {
            if let Ok(s) = session_map::spawn_meta(
                m,
                &st.theme_name,
                session_map::START_COLS,
                session_map::START_ROWS,
            ) {
                sess.map.insert(id, s);
            }
        }
        *dirty = true;
    }
}

pub fn apply_action(
    st: &mut AppState,
    sess: &mut SessionMap,
    ui: &mut UiState,
    action: Action,
    dirty: &mut bool,
) {
    let tab = st
        .win()
        .map(|w| w.tree.active_tab.min(w.tree.tabs.len().saturating_sub(1)))
        .unwrap_or(0);
    match action {
        Action::NewTab => {
            do_new_tab(st, sess, PaneKind::Local, dirty);
        }
        Action::NewWindow => {
            // Normally intercepted in keyboard.rs; kept here so any other
            // caller (IPC) gets the same behavior.
            crate::windows::spawn(st, sess, dirty);
        }
        Action::SplitHorizontal => {
            do_split(st, sess, tab, None, Axis::Horizontal, dirty);
        }
        Action::SplitVertical => {
            do_split(st, sess, tab, None, Axis::Vertical, dirty);
        }
        Action::SplitDefault => {
            let axis = st.settings.split_axis;
            do_split(st, sess, tab, None, axis, dirty);
        }
        Action::ClosePane => {
            if let Some(pane) = st
                .win()
                .and_then(|w| w.tree.tabs.get(tab).map(|t| t.focused))
            {
                do_close_pane(st, sess, ui, tab, pane, dirty);
            }
        }
        Action::CycleFocus(fwd) => {
            if let Some(w) = st.win_mut() {
                layout_tree::cycle_focus(&mut w.tree, tab, fwd);
            }
        }
        Action::PrevTab => {
            if tab > 0 {
                if let Some(w) = st.win_mut() {
                    w.tree.active_tab = tab - 1;
                    w.ui.zoom = false; // zoom is per-tab
                }
                *dirty = true;
            }
        }
        Action::NextTab => {
            if tab + 1 < st.win().map_or(0, |w| w.tree.tabs.len()) {
                if let Some(w) = st.win_mut() {
                    w.tree.active_tab = tab + 1;
                    w.ui.zoom = false; // zoom is per-tab
                }
                *dirty = true;
            }
        }
        Action::FocusUp => {
            if let Some(w) = st.win_mut() {
                layout_tree::move_focus(&mut w.tree, tab, layout_tree::FocusDir::Up);
            }
        }
        Action::FocusDown => {
            if let Some(w) = st.win_mut() {
                layout_tree::move_focus(&mut w.tree, tab, layout_tree::FocusDir::Down);
            }
        }
        Action::FocusLeft => {
            if let Some(w) = st.win_mut() {
                layout_tree::move_focus(&mut w.tree, tab, layout_tree::FocusDir::Left);
            }
        }
        Action::FocusRight => {
            if let Some(w) = st.win_mut() {
                layout_tree::move_focus(&mut w.tree, tab, layout_tree::FocusDir::Right);
            }
        }
        Action::ToggleZoom => {
            if let Some(w) = st.win_mut() {
                w.ui.zoom = !w.ui.zoom;
            }
        }
        Action::Respawn => {
            if let Some(pane) = st
                .win()
                .and_then(|w| w.tree.tabs.get(tab).map(|t| t.focused))
            {
                do_respawn(st, sess, pane, dirty);
            }
        }
        Action::Paste => {} // handled in input::keyboard with clipboard access
        Action::Quit => {}  // handled in input::keyboard (viewport close)
        Action::Copy => copy_focused(st, sess),
    }
}

/// Copy the focused pane's selection to the system clipboard.
/// Best-effort: empty selection or clipboard failure is silent.
pub fn copy_focused(st: &AppState, sess: &mut SessionMap) {
    let Some(pane) = st
        .win()
        .and_then(|w| w.tree.tabs.get(w.tree.active_tab).map(|t| t.focused))
    else {
        return;
    };
    let Some(s) = sess.map.get_mut(&pane) else {
        return;
    };
    match vt_pane::mouse::selection_text(s) {
        Ok(text) if !text.is_empty() => {
            if let Err(e) = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
                warn!("clipboard set: {e}");
            }
        }
        Ok(_) => {}
        Err(e) => warn!("selection text pane {pane}: {e}"),
    }
}

pub fn apply_pane_action(
    st: &mut AppState,
    sess: &mut SessionMap,
    ui: &mut UiState,
    tab: usize,
    pane: PaneId,
    action: PaneAction,
    dirty: &mut bool,
) {
    match action {
        PaneAction::SplitHorizontal => {
            do_split(st, sess, tab, Some(pane), Axis::Horizontal, dirty);
        }
        PaneAction::SplitVertical => {
            do_split(st, sess, tab, Some(pane), Axis::Vertical, dirty);
        }
        PaneAction::Close => {
            do_close_pane(st, sess, ui, tab, pane, dirty);
        }
        PaneAction::Respawn => {
            do_respawn(st, sess, pane, dirty);
        }
    }
}

#[cfg(test)]
mod close_tab_tests {
    use super::*;
    use crate::state::fresh_state;

    fn two_tab_state() -> (AppState, SessionMap, UiState) {
        let mut st = fresh_state();
        // fresh_state() has one empty tab; shape: tab0 split(1,2), tab1 pane(3)
        let tree = &mut st.windows[0].tree;
        tree.tabs.clear();
        tree.tabs.push(layout_tree::Tab {
            title: "style".into(),
            focused: 2,
            root: layout_tree::Node::Split {
                axis: Axis::Vertical,
                ratio: 0.5,
                first: Box::new(layout_tree::Node::Pane { id: 1 }),
                second: Box::new(layout_tree::Node::Pane { id: 2 }),
            },
        });
        tree.tabs.push(layout_tree::Tab {
            title: "extra".into(),
            focused: 3,
            root: layout_tree::Node::Pane { id: 3 },
        });
        tree.active_tab = 1;
        (st, session_map::session_map(), crate::state::ui_state())
    }

    #[test]
    fn close_pane_on_single_pane_tab_removes_tab() {
        let (mut st, mut sess, mut ui) = two_tab_state();
        let mut dirty = false;
        apply_action(&mut st, &mut sess, &mut ui, Action::ClosePane, &mut dirty);
        assert_eq!(
            st.windows[0].tree.tabs.len(),
            1,
            "extra tab must be removed"
        );
        assert!(!ui.quitting, "one tab still remains");
        assert!(!st.panes.contains_key(&3));
    }

    #[test]
    fn close_last_pane_quits() {
        let (mut st, mut sess, mut ui) = two_tab_state();
        st.windows[0].tree.tabs.remove(0);
        st.windows[0].tree.active_tab = 0;
        let mut dirty = false;
        apply_action(&mut st, &mut sess, &mut ui, Action::ClosePane, &mut dirty);
        assert!(st.windows[0].tree.tabs.is_empty());
        assert!(ui.quitting);
    }
}
