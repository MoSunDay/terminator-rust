//! Migration tests: a real pty hand-over both ways (adoption on the
//! receiver, a fake peer on the sender side) - the sender must never
//! signal a child it has already handed over, and a refused offer must
//! leave the local pane exactly as it was.

use super::*;
use ipc_proto::migrate::{self, MigrateNode, MigratePane, MigrateTab};
use ipc_proto::{PaneSelector, Response};
use layout_tree::PaneId;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;
use vt_pane::task as vtask;

use crate::session_map;
use crate::state::{data, fresh_state, tab_anchor, Data};

/// A fresh app whose single seeded pane runs a REAL shell (migration is
/// a pty hand-over, so the tests must not fake the session).
fn app_with_shell(echo: Option<&str>) -> (Data, PaneId) {
    let mut app = data(fresh_state(), Vec::new());
    let id = tab_anchor(&app.st.windows[0].tree.tabs[0]);
    let opts = vt_pane::SessionOpts::local_shell(80, 24);
    let mut sess = vtask::spawn_session(&opts).expect("spawn shell");
    if let Some(text) = echo {
        let _ = vtask::write(&sess, format!("echo {text}\n").as_bytes());
        assert!(wait_for_text(&mut sess, text), "shell echoed {text}");
    }
    session_map::note_spawned(&mut app.sess, id, sess);
    (app, id)
}

fn screen(sess: &mut vt_pane::Session) -> String {
    let frame = vtask::frame(sess).expect("frame");
    let mut out = String::new();
    for row in &frame.cells {
        for cell in row {
            out.push_str(&cell.text);
        }
        out.push('\n');
    }
    out
}

/// Poll the pane's screen for `needle` (terminal output is asynchronous).
fn wait_for_text(sess: &mut vt_pane::Session, needle: &str) -> bool {
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(20));
        let _ = vtask::pump(sess);
        if screen(sess).contains(needle) {
            return true;
        }
    }
    false
}

fn live(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("term-migrate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn leaf(kind: &str, pid: i32) -> MigrateNode {
    MigrateNode::Pane {
        pane: MigratePane {
            manual_title: Some("mig".to_string()),
            kind: kind.to_string(),
            degraded: false,
            pid,
            cols: 80,
            rows: 24,
        },
    }
}

/// Read `buf` exactly (the peer side of the wire, block by block).
fn read_exact(stream: &UnixStream, buf: &mut [u8]) {
    let mut src: &UnixStream = stream;
    src.read_exact(buf).expect("read exact");
}

#[test]
fn tab_offer_adopts_the_pty_and_plants_a_new_tab() {
    let mut app = data(fresh_state(), Vec::new());
    let mut src =
        vtask::spawn_session(&vt_pane::SessionOpts::local_shell(80, 24)).expect("spawn shell");
    let _ = vtask::write(&src, b"echo M-ADOPT\n");
    assert!(wait_for_text(&mut src, "M-ADOPT"), "shell echoed");
    let pid = vtask::child_pid(&src);
    let fd = vtask::dup_master(&src).expect("dup before the reader stops");
    vtask::stop_reader(&src);
    wait_reader(&src);
    let _ = vtask::pump(&mut src);
    let snap = vtask::snapshot(&mut src).expect("snapshot");
    let payload = migrate::encode_payload(&[snap]);
    let tab = MigrateTab {
        title: "handover".to_string(),
        focused: Some(0),
        root: leaf("local", pid),
    };

    let resp = tab_offer(&mut app, tab, &payload, vec![fd]);
    assert_eq!(resp, Response::Migrated { panes: 1 });

    // A new ACTIVE tab in the focused window, with fresh ids and the
    // manual title / meta carried over.
    assert_eq!(app.st.windows[0].tree.tabs.len(), 2);
    assert_eq!(app.st.windows[0].tree.active_tab, 1);
    assert_eq!(app.st.windows[0].tree.tabs[1].title, "handover");
    let new = tab_anchor(&app.st.windows[0].tree.tabs[1]);
    assert_ne!(new, 1, "migrated pane got a fresh id");
    assert_eq!(app.st.windows[0].tree.tabs[1].focused, new);
    assert_eq!(app.st.panes[&new].manual_title.as_deref(), Some("mig"));
    // The adopted session IS the same child, on the same pty: typing
    // into it reaches the shell that was running in the source pane.
    let sess = app.sess.map.get_mut(&new).expect("adopted session");
    assert_eq!(vtask::child_pid(sess), pid);
    assert!(
        wait_for_text(sess, "M-ADOPT"),
        "snapshot restored the screen"
    );
    let _ = vtask::write(sess, b"echo M-ADOPTED\n");
    assert!(wait_for_text(sess, "M-ADOPTED"), "adopted pty is live");
    session_map::terminate(&mut app.sess, new);
}

#[test]
fn tab_offer_rejects_a_mismatched_fd_count() {
    let mut app = data(fresh_state(), Vec::new());
    let tab = MigrateTab {
        title: "t".to_string(),
        focused: None,
        root: MigrateNode::Split {
            axis: "v".to_string(),
            ratio: 0.5,
            first: Box::new(leaf("local", 0)),
            second: Box::new(leaf("remote:h1", 0)),
        },
    };
    let payload = migrate::encode_payload(&[None, None]);
    let resp = tab_offer(&mut app, tab, &payload, Vec::new());
    assert!(matches!(resp, Response::Error { .. }), "got {resp:?}");
    assert_eq!(app.st.windows[0].tree.tabs.len(), 1, "nothing was planted");
    assert!(app.sess.map.is_empty());
}

#[test]
fn migrate_out_hands_the_pty_over_and_never_signals_the_child() {
    let dir = scratch("out");
    let path = dir.join("peer.sock");
    let listener = UnixListener::bind(&path).expect("bind");
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        // Header + fds ride the first message; the snapshot payload
        // follows as plain writes (block by block).
        let mut buf = [0u8; 8192];
        let (n, fds) = super::super::fd::recv_with_fds(&stream, &mut buf, 16).expect("recv");
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        let req: ipc_proto::Request = serde_json::from_str(head.trim_end()).expect("header line");
        let leaves = match &req {
            ipc_proto::Request::TabOffer { tab } => migrate::leaf_count(&tab.root),
            other => panic!("expected tab_offer, got {other:?}"),
        };
        let mut blocks = Vec::new();
        for _ in 0..leaves {
            let mut hdr = [0u8; 4];
            read_exact(&stream, &mut hdr);
            let len = u32::from_le_bytes(hdr) as usize;
            let mut block = vec![0u8; len];
            read_exact(&stream, &mut block);
            blocks.push(block);
        }
        let mut ack = serde_json::to_vec(&Response::Migrated { panes: leaves }).expect("ack");
        ack.push(b'\n');
        let mut sink = &stream;
        sink.write_all(&ack).expect("write ack");
        (leaves, blocks, fds)
    });

    let (mut app, id) = app_with_shell(Some("M-OUT"));
    let pid = vtask::child_pid(app.sess.map.get(&id).expect("session"));
    let target = path.to_string_lossy().to_string();
    let resp = migrate_out(&mut app, PaneSelector::Id(id), target);
    assert_eq!(resp, Response::Migrated { panes: 1 });

    let (leaves, blocks, fds) = peer.join().expect("peer");
    assert_eq!(leaves, 1);
    assert_eq!(fds.len(), 1, "the receiver holds the handed-over pty");
    assert!(!blocks[0].is_empty(), "the snapshot block carried state");
    // Local state is gone; the child is NOT (it now runs under the
    // receiver's fd - the sender must never signal it).
    assert!(app.st.windows[0].tree.tabs.is_empty());
    assert!(!app.st.panes.contains_key(&id));
    assert!(!app.sess.map.contains_key(&id));
    assert!(app.dirty);
    assert!(live(pid), "the sender must not signal a migrated child");
    drop(fds);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_offer_restores_the_pane_locally() {
    let dir = scratch("refuse");
    let path = dir.join("peer.sock");
    let listener = UnixListener::bind(&path).expect("bind");
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 8192];
        let _ = super::super::fd::recv_with_fds(&stream, &mut buf, 16);
        // No ack at all: the offer is considered lost.
    });

    let (mut app, id) = app_with_shell(Some("M-KEEP"));
    let pid = vtask::child_pid(app.sess.map.get(&id).expect("session"));
    let target = path.to_string_lossy().to_string();
    let resp = migrate_out(&mut app, PaneSelector::Id(id), target);
    assert!(matches!(resp, Response::Error { .. }), "got {resp:?}");
    peer.join().expect("peer");

    // Nothing left: the tab, the meta and the session are back, and the
    // restored terminal still shows what the pane displayed before.
    assert_eq!(app.st.windows[0].tree.tabs.len(), 1);
    assert!(app.st.panes.contains_key(&id));
    assert!(!app.dirty, "a failed migration does not dirty the state");
    let sess = app.sess.map.get_mut(&id).expect("re-adopted session");
    assert_eq!(vtask::child_pid(sess), pid);
    assert!(live(pid));
    assert!(
        wait_for_text(sess, "M-KEEP"),
        "screen restored from the snapshot"
    );
    let _ = vtask::write(sess, b"echo M-BACK\n");
    assert!(
        wait_for_text(sess, "M-BACK"),
        "the same child still answers"
    );
    session_map::terminate(&mut app.sess, id);
    let _ = std::fs::remove_dir_all(&dir);
}
