//! Receiver half: adopt each offered pty into a fresh session and
//! plant the tab in the user-focused window. A failure unwinds what
//! was already adopted WITHOUT signalling the children - their ptys
//! are still travelling back to the sender's rollback.

use std::os::fd::{IntoRawFd, OwnedFd};

use ipc_proto::migrate::{self, MigratePane, MigrateTab};
use ipc_proto::Response;
use layout_tree::{PaneId, Tab};
use vt_pane::task as vtask;
use vt_pane::PtyHandle;

use crate::render::colors;
use crate::session_map;
use crate::state::Data;

use super::wire::{build_node, meta_of};

/// Receiver half: adopt `tab` plus, per depth-first leaf, one descriptor in
/// `fds` and one snapshot block in `payload`, then plant it as a new active
/// tab of the user-focused window.
pub fn tab_offer(data: &mut Data, tab: MigrateTab, payload: &[u8], fds: Vec<OwnedFd>) -> Response {
    let n = migrate::leaf_count(&tab.root);
    if n == 0 || n > migrate::MAX_MIGRATE_PANES {
        return Response::err(format!(
            "migrate offer has {n} panes, expected 1..={}",
            migrate::MAX_MIGRATE_PANES
        ));
    }
    if fds.len() != n {
        return Response::err(format!(
            "migrate offer carries {} ptys for {n} panes",
            fds.len()
        ));
    }
    let snaps = match migrate::decode_payload(payload, n) {
        Ok(snaps) => snaps,
        Err(e) => return Response::err(format!("migrate payload: {e}")),
    };
    let Some(wi) = pick_window(data) else {
        return Response::err("no window available to receive the tab");
    };
    let MigrateTab {
        title,
        focused: focus_hint,
        root: wire,
    } = tab;
    // Fresh pane ids, allocated from the global budget so a later split in
    // any window can never collide with a migrated pane.
    let mut next = data
        .st
        .next_pane_id
        .max(layout_tree::next_pane_id(&data.st.windows[wi].tree));
    let mut plan: Vec<(PaneId, MigratePane)> = Vec::with_capacity(n);
    let root = build_node(wire, &mut next, &mut plan);
    let dark = colors::is_dark(&data.st.theme_name);
    let mut pending = fds.into_iter();
    let mut adopted: Vec<PaneId> = Vec::with_capacity(n);
    for (i, (id, mp)) in plan.iter().enumerate() {
        let Some(fd) = pending.next() else {
            return Response::err(format!("migrate offer is missing the pty for pane {id}"));
        };
        let handle = PtyHandle {
            master_fd: fd.into_raw_fd(),
            child_pid: mp.pid,
        };
        let opts = session_map::adopt_opts(mp.cols, mp.rows, dark);
        match vtask::adopt_session(handle, snaps.get(i).and_then(|s| s.as_deref()), &opts) {
            Ok(sess) => {
                data.st.panes.insert(*id, meta_of(mp, &data.registry));
                session_map::note_spawned(&mut data.sess, *id, sess);
                adopted.push(*id);
            }
            Err(e) => {
                let undone = drop_adopted(data, &adopted);
                return Response::err(format!("cannot adopt pane {id}: {e}{undone}"));
            }
        }
    }
    if adopted.len() != n {
        let undone = drop_adopted(data, &adopted);
        return Response::err(format!(
            "migrate offer adopted {} of {n} panes{undone}",
            adopted.len()
        ));
    }
    let focused = focus_hint
        .and_then(|i| plan.get(i as usize).map(|(id, _)| *id))
        .unwrap_or(adopted[0]);
    {
        let win = &mut data.st.windows[wi];
        layout_tree::ensure_next_pane_id(&mut win.tree, next);
        win.tree.tabs.push(Tab {
            title,
            root,
            focused,
        });
        win.tree.active_tab = win.tree.tabs.len() - 1;
        win.ui.zoom = false;
        crate::actions::prune_tab_edit(&win.tree, &mut win.ui);
    }
    data.st.collect_alloc();
    data.dirty = true;
    Response::Migrated { panes: n }
}

/// Index of the window a migrated tab should land in: the user-focused one
/// (clamped - a focus that outlived its window must not receive silently).
fn pick_window(data: &Data) -> Option<usize> {
    (!data.st.windows.is_empty()).then(|| data.st.focus.min(data.st.windows.len() - 1))
}

/// Drop sessions that were adopted into a half-built tab: detach (no
/// signal - the sender is about to re-adopt them) and forget the metas.
fn drop_adopted(data: &mut Data, adopted: &[PaneId]) -> String {
    for id in adopted {
        session_map::detach(&mut data.sess, *id);
        data.st.panes.remove(id);
    }
    if adopted.is_empty() {
        String::new()
    } else {
        format!(" (rolled back {})", adopted.len())
    }
}
