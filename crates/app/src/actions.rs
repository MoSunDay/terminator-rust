//! State transitions that coordinate the layout tree, pane metadata and
//! live sessions (spawn/close/respawn). Pure tree edits live in state.rs.

use layout_tree::{close_pane, close_tab, new_tab, Axis, PaneId};
use log::warn;
use remote::{PaneKind, EXIT_NO_ZELLIJ};

use crate::session_map::{self, SessionMap};
use crate::state::{self, new_pane_meta, Action, AppState, PaneAction, UiState};

/// Spawn sessions for every pane that lacks one; drop orphaned sessions.
pub fn ensure_sessions(st: &AppState, sess: &mut SessionMap) {
    let ids = state::all_pane_ids(&st.tree);
    sess.map.retain(|id, _| ids.contains(id));
    for id in ids {
        if sess.map.contains_key(&id) {
            continue;
        }
        spawn_pane(st, sess, id);
    }
}

fn spawn_pane(st: &AppState, sess: &mut SessionMap, id: PaneId) {
    if let Some(meta) = st.panes.get(&id) {
        match session_map::spawn_meta(meta, &st.theme_name, session_map::START_COLS, session_map::START_ROWS) {
            Ok(s) => {
                sess.map.insert(id, s);
            }
            Err(e) => warn!("spawn pane {id}: {e}"),
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
    let idx = new_tab(&mut st.tree, &title);
    let pane = st.tree.tabs.get(idx).map(|t| t.focused).unwrap_or(0);
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
    let target = pane.or_else(|| st.tree.tabs.get(tab).map(|t| t.focused))?;
    let new_id = state::split_tree_pane(st, tab, target, axis)?;
    spawn_pane(st, sess, new_id);
    *dirty = true;
    Some(new_id)
}

pub fn do_close_pane(
    st: &mut AppState,
    sess: &mut SessionMap,
    tab: usize,
    pane: PaneId,
    dirty: &mut bool,
) {
    if close_pane(&mut st.tree, tab, pane).is_some() {
        st.panes.remove(&pane);
        sess.map.remove(&pane);
        *dirty = true;
    }
    if st.tree.tabs.is_empty() {
        do_new_tab(st, sess, PaneKind::Local, dirty);
    }
}

pub fn do_close_tab(st: &mut AppState, sess: &mut SessionMap, tab: usize, dirty: &mut bool) {
    if let Some(t) = st.tree.tabs.get(tab) {
        for pane in layout_tree::sorted_pane_ids(&t.root) {
            st.panes.remove(&pane);
            sess.map.remove(&pane);
        }
    }
    close_tab(&mut st.tree, tab);
    if st.tree.tabs.is_empty() {
        do_new_tab(st, sess, PaneKind::Local, dirty);
    }
    *dirty = true;
}

/// Kill and respawn a pane's process with the same plan. Remotes go through
/// the idempotent bootstrap again (degraded flag reset).
pub fn do_respawn(st: &mut AppState, sess: &mut SessionMap, pane: PaneId, dirty: &mut bool) {
    let Some(meta) = st.panes.get_mut(&pane) else { return };
    meta.degraded = false;
    sess.map.remove(&pane);
    if let Some(meta) = st.panes.get(&pane) {
        match session_map::spawn_meta(meta, &st.theme_name, session_map::START_COLS, session_map::START_ROWS) {
            Ok(s) => {
                sess.map.insert(pane, s);
            }
            Err(e) => warn!("respawn pane {pane}: {e}"),
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
        if st.panes.get(&id).map(|m| m.degraded).unwrap_or(true) {
            continue;
        }
        if let Some(m) = st.panes.get_mut(&id) {
            m.degraded = true;
        }
        sess.map.remove(&id);
        if let Some(m) = st.panes.get(&id) {
            if let Ok(s) = session_map::spawn_meta(m, &st.theme_name, session_map::START_COLS, session_map::START_ROWS) {
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
    let tab = st.tree.active_tab.min(st.tree.tabs.len().saturating_sub(1));
    match action {
        Action::NewTab => {
            do_new_tab(st, sess, PaneKind::Local, dirty);
        }
        Action::SplitHorizontal => {
            do_split(st, sess, tab, None, Axis::Horizontal, dirty);
        }
        Action::SplitVertical => {
            do_split(st, sess, tab, None, Axis::Vertical, dirty);
        }
        Action::ClosePane => {
            if let Some(pane) = st.tree.tabs.get(tab).map(|t| t.focused) {
                do_close_pane(st, sess, tab, pane, dirty);
            }
            ui.zoom = false;
        }
        Action::CycleFocus(fwd) => {
            layout_tree::cycle_focus(&mut st.tree, tab, fwd);
        }
        Action::PrevTab => {
            if tab > 0 {
                st.tree.active_tab = tab - 1;
                *dirty = true;
            }
        }
        Action::NextTab => {
            if tab + 1 < st.tree.tabs.len() {
                st.tree.active_tab = tab + 1;
                *dirty = true;
            }
        }
        Action::FocusUp => {
            layout_tree::move_focus(&mut st.tree, tab, layout_tree::FocusDir::Up);
        }
        Action::FocusDown => {
            layout_tree::move_focus(&mut st.tree, tab, layout_tree::FocusDir::Down);
        }
        Action::FocusLeft => {
            layout_tree::move_focus(&mut st.tree, tab, layout_tree::FocusDir::Left);
        }
        Action::FocusRight => {
            layout_tree::move_focus(&mut st.tree, tab, layout_tree::FocusDir::Right);
        }
        Action::ToggleZoom => {
            ui.zoom = !ui.zoom;
        }
        Action::Respawn => {
            if let Some(pane) = st.tree.tabs.get(tab).map(|t| t.focused) {
                do_respawn(st, sess, pane, dirty);
            }
        }
        Action::CopyNoop => {} // TODO: selection copy once selection exists
    }
}

pub fn apply_pane_action(
    st: &mut AppState,
    sess: &mut SessionMap,
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
            do_close_pane(st, sess, tab, pane, dirty);
        }
        PaneAction::Respawn => {
            do_respawn(st, sess, pane, dirty);
        }
    }
}
