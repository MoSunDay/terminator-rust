//! State transitions that coordinate the layout tree, pane metadata and
//! live sessions (spawn/close/respawn). Pure tree edits live in state.rs.

use std::time::Instant;

use layout_tree::{
    close_pane, close_tab, move_pane_to_pane, new_tab, sorted_pane_ids, Axis, DropZone, PaneId,
};
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
                sess.exited_seen.remove(&id);
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

/// Drop of the Ctrl+drag pane move: `pane` lands next to `target`
/// (edge split) or swaps with it (Center). Sessions and pane meta are
/// keyed by pane id, so this is pure tree surgery + dirty flag.
pub fn do_move_pane(
    st: &mut AppState,
    tab: usize,
    pane: PaneId,
    target: PaneId,
    zone: DropZone,
    dirty: &mut bool,
) {
    let wi = st.active_idx();
    let ratio = st.settings.split_ratio;
    let moved = st
        .windows
        .get_mut(wi)
        .is_some_and(|w| move_pane_to_pane(&mut w.tree, tab, pane, target, zone, ratio));
    if moved {
        if let Some(w) = st.windows.get_mut(wi) {
            w.ui.zoom = false;
        }
        *dirty = true;
    }
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
                sess.exited_seen.remove(&pane);
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

/// A pane whose session finished and whose exit has been visible for at
/// least [`session_map::EXIT_GRACE`]. Pure predicate for [`close_exited`].
fn exit_ripe(sess: &SessionMap, id: &PaneId, now: Instant) -> bool {
    let exited = match sess.map.get(id) {
        // alive, or auto_degrade owns the exit-42 marker
        Some(s) => s.exit.is_some_and(|e| e != EXIT_NO_ZELLIJ),
        // spawn-backoff candidate, not a corpse
        None => false,
    };
    exited
        && sess
            .exited_seen
            .get(id)
            .is_some_and(|seen| now.duration_since(*seen) >= session_map::EXIT_GRACE)
}

/// Close panes whose session has exited: the shell is gone, so its pane
/// goes too instead of lingering as a corpse the user must click away.
/// Closing the last pane keeps the existing semantics - the only window
/// quits the app, a secondary removes its OS window, the root with living
/// siblings respawns a fresh tab next frame. `st.active` is restored.
pub fn close_exited(st: &mut AppState, sess: &mut SessionMap, ui: &mut UiState, dirty: &mut bool) {
    let now = Instant::now();
    for (id, s) in sess.map.iter() {
        if s.exit.is_some() && !sess.exited_seen.contains_key(id) {
            sess.exited_seen.insert(*id, now);
        }
    }
    let prev_active = st.active;
    loop {
        let hit = st.windows.iter().enumerate().find_map(|(wi, w)| {
            w.tree.tabs.iter().enumerate().find_map(|(ti, t)| {
                sorted_pane_ids(&t.root)
                    .into_iter()
                    .find(|id| exit_ripe(sess, id, now))
                    .map(|id| (wi, ti, id))
            })
        });
        let Some((wi, ti, pane)) = hit else {
            break;
        };
        st.active = wi;
        do_close_pane(st, sess, ui, ti, pane, dirty);
    }
    st.active = prev_active;
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

#[cfg(test)]
mod move_pane_tests {
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
}

#[cfg(test)]
mod close_exited_tests {
    use super::*;
    use crate::state::{fresh_state, split_tree_pane, ui_state};
    use std::time::Duration;
    use vt_pane::{task as vtask, SessionOpts};

    /// A real short-lived session that has already exited.
    fn dead_session(status: i32) -> vt_pane::Session {
        let code = status.to_string();
        let opts = SessionOpts {
            cols: 10,
            rows: 5,
            argv: vec!["/bin/sh".into(), "-c".into(), format!("exit {code}")],
            env: Vec::new(),
            scrollback_lines: 100,
            dark: true,
        };
        let mut s = vtask::spawn_session(&opts).expect("spawn sh");
        for _ in 0..250 {
            let _ = vtask::pump(&mut s);
            if s.exit.is_some() {
                assert_eq!(s.exit, Some(status));
                return s;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("test session never exited");
    }

    /// One window, one tab, panes 1|2 split, both with live meta.
    fn split_state() -> (AppState, SessionMap, UiState) {
        let mut st = fresh_state();
        let _ = split_tree_pane(&mut st, 0, 1, Axis::Vertical);
        (st, session_map::session_map(), ui_state())
    }

    /// Mark a pane's session as exited past the grace window.
    fn ripen(sess: &mut SessionMap, pane: PaneId, status: i32) {
        sess.map.insert(pane, dead_session(status));
        sess.exited_seen.insert(
            pane,
            Instant::now() - session_map::EXIT_GRACE - Duration::from_millis(50),
        );
    }

    #[test]
    fn exited_pane_closes_after_grace_and_keeps_sibling() {
        let (mut st, mut sess, mut ui) = split_state();
        let mut dirty = false;
        ripen(&mut sess, 1, 0);
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        assert!(dirty, "state change must be persisted");
        assert!(!st.panes.contains_key(&1), "corpse pane removed");
        assert!(!sess.map.contains_key(&1), "corpse session removed");
        assert!(!sess.exited_seen.contains_key(&1));
        assert!(st.panes.contains_key(&2), "sibling untouched");
        assert_eq!(st.windows[0].tree.tabs.len(), 1);
        assert!(!ui.quitting);
        assert_eq!(st.active, 0, "active pointer restored");
    }

    #[test]
    fn exit_within_grace_lingers_one_cycle() {
        let (mut st, mut sess, mut ui) = split_state();
        let mut dirty = false;
        sess.map.insert(1, dead_session(0)); // first sight: no seen-marker yet
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        assert!(
            st.panes.contains_key(&1),
            "pane must survive the first frame after the exit"
        );
        assert!(sess.exited_seen.contains_key(&1), "exit was timestamped");
        // Instantly die + instantly close would fork-loop the respawn.
    }

    #[test]
    fn last_pane_exit_quits_the_app() {
        let (mut st, mut sess, mut ui) = split_state();
        let mut dirty = false;
        // remove pane 2's tab membership by closing via the tree directly
        let _ = layout_tree::close_pane(&mut st.windows[0].tree, 0, 2);
        st.panes.remove(&2);
        ripen(&mut sess, 1, 0);
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        assert!(st.windows[0].tree.tabs.is_empty());
        assert!(ui.quitting, "only window + last pane => quit");
    }

    #[test]
    fn exit42_marker_is_left_for_auto_degrade() {
        let (mut st, mut sess, mut ui) = split_state();
        let mut dirty = false;
        ripen(&mut sess, 1, EXIT_NO_ZELLIJ);
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        assert!(st.panes.contains_key(&1), "degrade path owns exit 42");
        assert!(sess.map.contains_key(&1));
    }

    #[test]
    fn secondary_last_exit_removes_only_that_window() {
        let (mut st, mut sess, mut ui) = split_state();
        let mut dirty = false;
        crate::windows::spawn(&mut st, &mut sess, &mut dirty);
        assert_eq!(st.windows.len(), 2);
        let gone = layout_tree::sorted_pane_ids(&st.windows[1].tree.tabs[0].root)[0];
        ripen(&mut sess, gone, 0);
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        assert_eq!(st.windows.len(), 1, "secondary removed itself");
        assert!(!ui.quitting, "root keeps the app alive");
        assert!(st.panes.contains_key(&1) && st.panes.contains_key(&2));
        assert_eq!(st.focus, 0, "focus retargeted to root");
    }
}
