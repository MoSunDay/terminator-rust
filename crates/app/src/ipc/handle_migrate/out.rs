//! Sender half: duplicate every master fd, quiesce and snapshot the
//! tab, offer it to the peer and touch local state only on a positive
//! ack. A lost or refused offer re-adopts the panes from the dups we
//! still hold, so it is a no-op for the local session.

use std::io::{Read, Write};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use ipc_proto::migrate::{self, MigrateTab};
use ipc_proto::{PaneSelector, Response};
use layout_tree::PaneId;
use vt_pane::task as vtask;
use vt_pane::PtyHandle;

use crate::actions::winops;
use crate::render::colors;
use crate::session_map;
use crate::state::Data;

use super::wire::{focus_index, tab_title, wire_node};
use super::{Leaf, IO_TIMEOUT, MAX_LINE, READER_JOIN};

/// Sender half: hand the tab containing `pane` to the instance listening
/// on `target` (a control-socket path). On success the local tab goes away
/// while its children keep running under the target's control.
pub fn migrate_out(data: &mut Data, pane: PaneSelector, target: String) -> Response {
    let id = match super::super::handle::resolve(&data.st, &pane) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let Some((wi, ti)) = locate(&data.st, id) else {
        return Response::err(format!("pane {id} is not in any tab"));
    };
    let leaves = match collect_leaves(&data.st, &mut data.sess, wi, ti) {
        Ok(leaves) => leaves,
        Err(e) => return Response::err(e),
    };
    if leaves.len() > migrate::MAX_MIGRATE_PANES {
        return Response::err(format!(
            "tab has {} panes, at most {} travel per offer",
            leaves.len(),
            migrate::MAX_MIGRATE_PANES
        ));
    }
    // 1. Duplicate every master while the local readers still own the fds.
    let mut fds: Vec<OwnedFd> = Vec::with_capacity(leaves.len());
    for l in &leaves {
        let Some(s) = data.sess.map.get(&l.id) else {
            return Response::err(format!("pane {} has no live session to migrate", l.id));
        };
        match vtask::dup_master(s) {
            Ok(fd) => fds.push(fd),
            Err(e) => {
                return Response::err(format!("pane {}: cannot duplicate pty master: {e}", l.id));
            }
        }
    }
    // 2. Quiesce, then encode. The dup above is independent of the reader
    //    thread's close, so stopping it costs the local pane nothing that
    //    the snapshot does not carry back.
    for l in &leaves {
        if let Some(s) = data.sess.map.get(&l.id) {
            vtask::stop_reader(s);
        }
    }
    let mut snaps: Vec<Option<Vec<u8>>> = Vec::with_capacity(leaves.len());
    for l in &leaves {
        let snap = match data.sess.map.get_mut(&l.id) {
            Some(s) => {
                wait_reader(s);
                if let Err(e) = vtask::pump(s) {
                    log::debug!("migrate: final pump of pane {}: {e}", l.id);
                }
                vtask::snapshot(s).unwrap_or_else(|e| {
                    log::warn!("migrate: snapshot of pane {} failed: {e}", l.id);
                    None
                })
            }
            None => None,
        };
        snaps.push(snap);
    }
    // 3. Offer: header line + fds + payload in one sendmsg.
    let mut next = 0usize;
    let wire = MigrateTab {
        title: tab_title(&data.st, wi, ti),
        focused: focus_index(&data.st, wi, ti, &leaves),
        root: wire_node(&data.st, wi, ti, &leaves, &mut next),
    };
    let mut head = match serde_json::to_vec(&ipc_proto::Request::TabOffer { tab: wire }) {
        Ok(head) => head,
        Err(e) => {
            let failed = rollback(data, &leaves, &fds, &snaps);
            return Response::err(format!("cannot serialize the offer: {e}{failed}"));
        }
    };
    head.push(b'\n');
    let mut payload = migrate::encode_payload(&snaps);
    if head.len() + payload.len() > migrate::MAX_MIGRATE_PAYLOAD {
        log::warn!(
            "migrate: {} bytes of terminal state exceed the {} cap; the panes travel without their screens",
            payload.len(),
            migrate::MAX_MIGRATE_PAYLOAD
        );
        payload = migrate::encode_payload(&vec![None; leaves.len()]);
    }
    match offer(&target, &head, &payload, &fds) {
        Ok(panes) => {
            commit(data, wi, ti, &leaves);
            Response::Migrated { panes }
        }
        Err(e) => Response::err(format!("{e}{}", rollback(data, &leaves, &fds, &snaps))),
    }
}

/// Window/tab indices of the tab containing `id`.
fn locate(st: &crate::state::AppState, id: PaneId) -> Option<(usize, usize)> {
    for (wi, win) in st.windows.iter().enumerate() {
        for (ti, tab) in win.tree.tabs.iter().enumerate() {
            let mut ids = Vec::new();
            layout_tree::pane_ids(&tab.root, &mut ids);
            if ids.contains(&id) {
                return Some((wi, ti));
            }
        }
    }
    None
}

/// The tab's panes in depth-first order, with the facts the wire needs:
/// the pty metrics come from the live session, so the receiver can size the
/// adopted terminal and answer size queries before its first sync_frame.
/// A pane without a session or a meta cannot travel (its pty would be
/// missing on the far side), so the whole offer is refused.
fn collect_leaves(
    st: &crate::state::AppState,
    sess: &mut session_map::SessionMap,
    wi: usize,
    ti: usize,
) -> Result<Vec<Leaf>, String> {
    let Some(tab) = st.windows.get(wi).and_then(|w| w.tree.tabs.get(ti)) else {
        return Err("nothing to migrate".to_string());
    };
    let mut ids = Vec::new();
    layout_tree::pane_ids(&tab.root, &mut ids);
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(meta) = st.panes.get(&id) else {
            return Err(format!("pane {id} has no configuration to migrate"));
        };
        let Some(s) = sess.map.get_mut(&id) else {
            return Err(format!("pane {id} has no live session to migrate"));
        };
        let (cols, rows) = match vtask::frame(s) {
            Ok(f) => (f.cols, f.rows),
            Err(_) => (session_map::START_COLS, session_map::START_ROWS),
        };
        out.push(Leaf {
            id,
            meta: meta.clone(),
            pid: vtask::child_pid(s),
            cols,
            rows,
        });
    }
    Ok(out)
}

/// Block until a stopped reader thread has closed its master fd (bounded).
pub fn wait_reader(sess: &vt_pane::Session) {
    let deadline = Instant::now() + READER_JOIN;
    while !vtask::reader_done(sess) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Send the offer and read the ack. `Ok(panes)` means the target adopted
/// all of them and owns their fds from here on.
///
/// The fds ride the header message (SCM_RIGHTS attaches to the first byte
/// only); the snapshot payload follows as plain writes, so a multi-megabyte
/// scrollback is not bound by the socket buffer. The receiver is
/// length-driven, so it needs no framing beyond the header.
fn offer(target: &str, head: &[u8], payload: &[u8], fds: &[OwnedFd]) -> Result<usize, String> {
    let stream = UnixStream::connect(target).map_err(|e| format!("connect {target}: {e}"))?;
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    super::super::fd::send_with_fds(&stream, head, fds).map_err(|e| format!("send offer: {e}"))?;
    let mut sink = &stream;
    sink.write_all(payload)
        .map_err(|e| format!("send snapshots: {e}"))?;
    match read_ack(&stream) {
        Some(Response::Migrated { panes }) => Ok(panes),
        Some(Response::Error { message }) => Err(format!("target refused the offer: {message}")),
        Some(other) => Err(format!("target replied unexpectedly: {other:?}")),
        None => Err("no ack from the target instance".to_string()),
    }
}

fn read_ack(stream: &UnixStream) -> Option<Response> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut src: &UnixStream = stream;
    while buf.len() <= MAX_LINE {
        match src.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                log::warn!("migrate: reading the ack failed: {e}");
                return None;
            }
        }
    }
    let line = std::str::from_utf8(&buf).ok()?;
    serde_json::from_str(line.trim_end()).ok()
}

/// Local half of a successful hand-over: the tab and its pane metas go
/// away, its sessions are DETACHED (never terminated - their children keep
/// running under the receiver) and an emptied secondary window removes
/// itself.
fn commit(data: &mut Data, wi: usize, ti: usize, leaves: &[Leaf]) {
    winops::take_tab(&mut data.st, wi, ti);
    for l in leaves {
        session_map::detach(&mut data.sess, l.id);
        data.st.panes.remove(&l.id);
    }
    winops::drop_empty_window(&mut data.st, wi);
    data.st.collect_alloc();
    data.dirty = true;
}

/// The offer did not land: re-adopt each pane from the dup we still hold
/// (its local session was unwound by the snapshot step) and restore the
/// snapshot we just encoded, so the pane comes back exactly as it was.
/// Returns a suffix naming the panes that could NOT be restored (their
/// children keep running, unreachable).
fn rollback(
    data: &mut Data,
    leaves: &[Leaf],
    fds: &[OwnedFd],
    snaps: &[Option<Vec<u8>>],
) -> String {
    let dark = colors::is_dark(&data.st.theme_name);
    let mut failed = Vec::new();
    for (i, l) in leaves.iter().enumerate() {
        let Some(dup) = fds.get(i).and_then(|fd| fd.try_clone().ok()) else {
            failed.push(format!("{}", l.id));
            continue;
        };
        session_map::detach(&mut data.sess, l.id);
        let handle = PtyHandle {
            master_fd: dup.into_raw_fd(),
            child_pid: l.pid,
        };
        let opts = session_map::adopt_opts(l.cols, l.rows, dark);
        match vtask::adopt_session(handle, snaps.get(i).and_then(|s| s.as_deref()), &opts) {
            Ok(sess) => session_map::note_spawned(&mut data.sess, l.id, sess),
            Err(e) => failed.push(format!("{}: {e}", l.id)),
        }
    }
    if failed.is_empty() {
        String::new()
    } else {
        format!("; panes lost locally: {}", failed.join(", "))
    }
}
