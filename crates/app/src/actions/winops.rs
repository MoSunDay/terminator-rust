//! Cross-OS-window tab surgery: move a tab into another window, merge
//! every window into the root, and the Ctrl+Shift+J "send to next
//! window" gesture.
//!
//! Sessions and PaneMeta are GLOBAL by pane id, so moving a tab between
//! windows is pure state surgery - the panes keep rendering from their
//! live sessions in the new window. Never call
//! [`crate::windows::remove_window`] for these paths (it terminates the
//! panes); drop the emptied WindowState directly instead.

use egui::Pos2;
use layout_tree::{ensure_next_pane_id, next_pane_id, PaneId, Tab};

use crate::session_map::SessionMap;
use crate::state::{self, AppState, UiState, WinScreens, WindowState};

/// Move the tab identified by its anchor pane id into another window.
/// Pure state surgery: sessions/PaneMeta stay (global by pane id). An
/// emptied source secondary window removes itself; the root keeps its
/// viewport (the screen() respawn branch plants a fresh tab when it
/// empties).
pub fn do_move_tab_to_window(
    st: &mut AppState,
    dirty: &mut bool,
    anchor: PaneId,
    dst_win: u64,
) -> bool {
    let src = st
        .windows
        .iter()
        .position(|w| w.tree.tabs.iter().any(|t| state::tab_anchor(t) == anchor));
    let (Some(src), Some(dst)) = (src, st.windows.iter().position(|w| w.id == dst_win)) else {
        return false;
    };
    if src == dst {
        return false;
    }
    let tab_idx = st.windows[src]
        .tree
        .tabs
        .iter()
        .position(|t| state::tab_anchor(t) == anchor)
        .unwrap_or(0);
    // Detach from the source, keeping its active_tab valid (the same
    // fixups layout_tree::close_tab applies).
    let Some(tab) = take_tab(st, src, tab_idx) else {
        return false;
    };
    // Append in the destination and make it the active tab there. The
    // destination allocator is seeded from the global budget FIRST so a
    // later split/new-tab in that window can never reuse a moved pane id.
    let budget = st.next_pane_id;
    {
        let dstw = &mut st.windows[dst];
        ensure_next_pane_id(&mut dstw.tree, budget);
        dstw.tree.tabs.push(tab);
        dstw.tree.active_tab = dstw.tree.tabs.len() - 1;
        dstw.ui.zoom = false;
        super::prune_tab_edit(&dstw.tree, &mut dstw.ui);
    }
    st.collect_alloc();
    // An emptied SECONDARY removes itself - without terminating anything,
    // its panes now live in the destination. The root keeps its viewport;
    // screen() respawns a fresh tab when the root empties.
    drop_empty_window(st, src);
    st.focus = st.windows.iter().position(|w| w.id == dst_win).unwrap_or(0);
    *dirty = true;
    true
}

/// Remove tab `idx` from window `wi`, keeping that window's `active_tab`
/// valid (the fixups `layout_tree::close_tab` applies) and returning the
/// detached tab. The window list is left alone - an emptied window is
/// [`drop_empty_window`]'s job - and so are the panes: sessions and
/// PaneMeta are global by pane id, so both move/close decisions stay
/// with the caller.
pub(crate) fn take_tab(st: &mut AppState, wi: usize, idx: usize) -> Option<Tab> {
    let win = st.windows.get_mut(wi)?;
    if idx >= win.tree.tabs.len() {
        return None;
    }
    let tab = win.tree.tabs.remove(idx);
    if win.tree.active_tab > idx {
        win.tree.active_tab -= 1;
    }
    let WindowState { tree, ui: wui, .. } = win;
    tree.active_tab = tree.active_tab.min(tree.tabs.len().saturating_sub(1));
    super::prune_tab_edit(tree, wui);
    Some(tab)
}

/// Drop window `wi` when `take_tab` emptied it. SECONDARY windows remove
/// themselves (root keeps its viewport: the screen() respawn branch
/// plants a fresh tab when the root empties). Pure state surgery - never
/// `windows::remove_window`, which would terminate panes that still live
/// somewhere else.
pub(crate) fn drop_empty_window(st: &mut AppState, wi: usize) {
    if wi > 0 && st.windows.get(wi).is_some_and(|w| w.tree.tabs.is_empty()) {
        st.windows.remove(wi);
        st.retarget_after_remove(wi);
    }
}

/// Ctrl+Shift+M: merge every other window's tabs into the ROOT window
/// (idx 0) and drop those windows WITHOUT terminating their panes.
/// Order: root tabs first, then each window's tabs in window order. The
/// root's active tab keeps pointing at the SAME tab.
pub fn do_merge_windows(st: &mut AppState, dirty: &mut bool) {
    if st.windows.len() < 2 {
        return;
    }
    let active_anchor = st.windows[0]
        .tree
        .tabs
        .get(st.windows[0].tree.active_tab)
        .map(state::tab_anchor);
    // Seed the root allocator above every window's budget BEFORE the
    // tabs land, then re-collect so the global counter stays >= all.
    let max_needed = st
        .windows
        .iter()
        .map(|w| next_pane_id(&w.tree))
        .max()
        .unwrap_or(2);
    let mut taken: Vec<Tab> = Vec::new();
    for w in st.windows.iter_mut().skip(1) {
        taken.append(&mut w.tree.tabs);
    }
    st.windows.truncate(1);
    {
        let root = &mut st.windows[0];
        ensure_next_pane_id(&mut root.tree, max_needed);
        root.tree.tabs.append(&mut taken);
        if let Some(a) = active_anchor {
            if let Some(idx) = root
                .tree
                .tabs
                .iter()
                .position(|t| state::tab_anchor(t) == a)
            {
                root.tree.active_tab = idx;
            }
        }
    }
    st.collect_alloc();
    st.focus = 0;
    st.active = 0;
    *dirty = true;
}

/// Ctrl+Shift+J: move the FOCUSED window's active tab to the NEXT window
/// (cyclic). With a single window, spawn one first (it arrives focused,
/// seeded with one shell tab), move the tab there, then close the seed
/// tab through the regular close path so its shell pane terminates
/// cleanly - net result: a new window holding exactly the moved tab,
/// focused.
pub fn do_move_tab_next_window(
    st: &mut AppState,
    sess: &mut SessionMap,
    ui: &mut UiState,
    dirty: &mut bool,
) {
    // The SOURCE is the user-focused window (`st.focus`), not the window
    // currently being rendered: key actions run for the focused viewport
    // and the destination below is picked from `st.focus` too.
    let Some(anchor) = st
        .windows
        .get(st.focus)
        .and_then(|w| w.tree.tabs.get(w.tree.active_tab))
        .map(state::tab_anchor)
    else {
        return;
    };
    let len = st.windows.len();
    if len == 0 {
        return;
    }
    if len == 1 {
        crate::windows::spawn(st, sess, dirty);
        let Some(dst) = st.windows.get(st.focus).map(|w| w.id) else {
            return;
        };
        if !do_move_tab_to_window(st, dirty, anchor, dst) {
            return;
        }
        // The seed tab (index 0) served its purpose: close it through the
        // regular path so its shell session terminates properly.
        // do_close_tab acts on st.active, so aim it at the destination.
        let saved_active = st.active;
        st.active = st.focus;
        super::do_close_tab(st, sess, ui, 0, dirty);
        st.active = saved_active;
        return;
    }
    let dst = st.windows[(st.focus + 1) % len].id;
    do_move_tab_to_window(st, dirty, anchor, dst);
}

/// egui viewport of a window id: `windows[0]` renders in the ROOT pass,
/// every secondary in `ViewportId(Id::new(id))` (windows.rs). Viewport
/// commands (Focus/Close) go to the id, never to the current pass.
pub(crate) fn viewport_of(st: &AppState, win_id: u64) -> egui::ViewportId {
    match st.windows.first() {
        Some(w) if w.id == win_id => egui::ViewportId::ROOT,
        _ => egui::ViewportId(egui::Id::new(win_id)),
    }
}

/// Which OTHER window's tab strip (screen points) contains `pos`?
/// `order` lists the live window ids in `st.windows` order: overlapping
/// strips resolve to the LAST match (secondaries render after the root,
/// so they read as on top), and stale published rects never match
/// because their ids have left `order`.
pub fn strip_hit(screens: &WinScreens, source: u64, pos: Pos2, order: &[u64]) -> Option<u64> {
    order
        .iter()
        .rev()
        .filter(|id| **id != source)
        .find_map(|id| {
            screens
                .strip
                .get(id)
                .filter(|r| r.contains(pos))
                .map(|_| *id)
        })
}

#[cfg(test)]
#[path = "winops_tests.rs"]
mod tests;
