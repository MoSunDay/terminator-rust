//! Network-level integration test for the remote reconnect path.
//!
//! A REAL localhost ssh + zellij connection is spawned through the app's
//! own session plumbing, then killed from the SERVER side (the
//! per-connection sshd child ONLY - never the listener: the test process
//! itself usually runs inside an sshd session). The app frame loop
//! (`pump_all` -> `close_exited` -> `reconnect::pump` ->
//! `ensure_sessions`) must then keep the pane, reattach the surviving
//! zellij session and restore its on-screen state.
//!
//! Run manually: `cargo test -p app reconnect_net -- --ignored --nocapture`.
//! Requires local sshd with key auth and zellij on PATH.

use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use layout_tree::PaneId;
use remote::{is_disconnect, PaneKind, RemoteTarget};
use vt_pane::task as vtask;
use vt_pane::Frame;

use super::reconnect;
use super::{close_exited, ensure_sessions};
use crate::session_map::{pump_all, session_map, terminate, SessionMap};
use crate::state::{fresh_state, new_pane_meta, ui_state};

/// Simulated main-loop cadence (~1 frame per iteration).
const FRAME_STEP: Duration = Duration::from_millis(100);

/// Concatenated cell text of a frame (row-major).
fn frame_text(f: &Frame) -> String {
    f.cells
        .iter()
        .flat_map(|row| row.iter())
        .map(|c| c.text.as_str())
        .collect()
}

/// TCP port of an `ss` address field (`127.0.0.1:54321` -> 54321).
fn port_of(field: &str) -> Option<u16> {
    field.rsplit(':').next()?.parse().ok()
}

/// pid named by the first `pid=N` digit run in an `ss -tnp` users blob.
fn line_pid(line: &str) -> Option<u32> {
    let digits: String = line
        .split("pid=")
        .nth(1)?
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Snapshot of `ss -tnp` lines (empty when ss is unavailable).
fn ss_lines() -> Vec<String> {
    Command::new("ss")
        .arg("-tnp")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Local port of the ssh CLIENT `client_pid`'s outbound connection.
fn client_port(lines: &[String], client_pid: i32) -> Option<u16> {
    let line = lines
        .iter()
        .find(|l| l.contains(&format!("pid={client_pid},")))?;
    port_of(line.split_whitespace().nth(3)?)
}

/// pid of the per-connection sshd whose socket is local :22 with peer
/// port `peer` (the server side of our client's connection).
fn server_sshd_pid(lines: &[String], peer: u16) -> Option<u32> {
    lines
        .iter()
        .filter(|l| l.starts_with("ESTAB") && l.contains("(\"sshd"))
        .find(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            f.len() >= 5 && port_of(f[3]) == Some(22) && port_of(f[4]) == Some(peer)
        })
        .and_then(|l| line_pid(l))
}

/// Kill the server side of the pane's ssh connection (per-connection
/// sshd child only, so the listener - and the test's own session -
/// survives). True when a server pid was found and signalled.
fn kill_server_side(client_pid: i32) -> bool {
    let lines = ss_lines();
    let server = client_port(&lines, client_pid).and_then(|p| server_sshd_pid(&lines, p));
    server.is_some_and(|pid| {
        Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Type a line into the pane's terminal and press Enter.
fn send_line(sess: &mut vt_pane::Session, line: &str) {
    let _ = vtask::paste(sess, line);
    let _ = vtask::send_key(sess, |ev| {
        ev.set_action(libghostty_vt::key::Action::Press);
        ev.set_key(libghostty_vt::key::Key::Enter);
    });
}

/// Delete the test's zellij session on the "remote" host (localhost).
fn delete_zellij_session(name: &str) {
    let _ = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "127.0.0.1",
            "zellij",
            "delete-session",
            name,
            "--force",
        ])
        .output();
}

/// Current frame text of pane `id` ("" when it has no live session).
fn pane_text(sess: &mut SessionMap, id: PaneId) -> String {
    match sess.map.get_mut(&id) {
        Some(s) if s.exit.is_none() => vtask::frame(s).map(|f| frame_text(&f)).unwrap_or_default(),
        _ => String::new(),
    }
}

/// True once the zellij attach gate cleared: content rendered and the
/// loading / terminal-query screens are gone (same gate as the remote
/// crate e2e).
fn gate_cleared(sess: &mut SessionMap) -> bool {
    let text = pane_text(sess, 1);
    !text.trim().is_empty()
        && !text.contains("Loading Zellij")
        && !text.contains("Querying terminal emulator")
}

/// A server-side kill of the pane's ssh connection must NOT let
/// `close_exited` destroy the pane: the zellij session survives on the
/// host, the reconnect pump respawns the ssh attach through the same
/// frame loop the real app runs, and the previous on-screen state
/// (echoed marker) is restored.
#[test]
#[ignore = "needs local sshd + zellij"]
fn server_reset_pump_reattaches_and_close_exited_spares_pane() {
    let pid = std::process::id();
    let zsession = format!("zt-pump-{pid}");
    let marker = format!("ZPMP{pid}");
    let target = RemoteTarget {
        label: "pump-net".into(),
        host: "127.0.0.1".into(),
        user: None,
        port: None,
        session_name: zsession.clone(),
    };
    let mut st = fresh_state();
    st.panes.insert(1, new_pane_meta(PaneKind::Remote(target)));
    let mut sess = session_map();
    let mut ui = ui_state();
    ensure_sessions(&st, &mut sess);
    assert!(
        sess.map.contains_key(&1),
        "remote pane spawned no ssh session"
    );

    // Wait for the zellij attach gate (loading screen cleared).
    let mut gated = false;
    let gate_at = Instant::now() + Duration::from_secs(10);
    while Instant::now() < gate_at {
        pump_all(&mut sess);
        if gate_cleared(&mut sess) {
            gated = true;
            break;
        }
        sleep(FRAME_STEP);
    }
    assert!(gated, "zellij attach gate did not clear within 10s");

    // Echo a unique marker so reattachment can prove state recovery.
    if let Some(s) = sess.map.get_mut(&1) {
        send_line(s, &format!("echo {marker}"));
    }
    let mut echoed = false;
    let echo_at = Instant::now() + Duration::from_secs(10);
    while Instant::now() < echo_at {
        pump_all(&mut sess);
        if pane_text(&mut sess, 1).contains(&marker) {
            echoed = true;
            break;
        }
        sleep(FRAME_STEP);
    }
    assert!(echoed, "marker {marker} never round-tripped through zellij");

    let old_pid = match sess.map.get(&1) {
        Some(s) => vtask::child_pid(s),
        None => 0,
    };
    assert!(old_pid > 0, "pane 1 has no ssh child pid");
    assert!(
        kill_server_side(old_pid),
        "no server-side sshd found for the pane's ssh client (pid {old_pid})"
    );

    // The app's frame loop, replayed at the real cadence: observe the
    // disconnect, keep the pane, back off, respawn, recover.
    let mut saw_disconnect = false;
    let mut saw_pending = false;
    let mut respawn_pid: Option<i32> = None;
    let mut recovered = false;
    let mut pane_kept = true;
    let mut dirty = false;
    let reset_at = Instant::now() + Duration::from_secs(25);
    while Instant::now() < reset_at {
        pump_all(&mut sess);
        if sess
            .map
            .get(&1)
            .is_some_and(|s| s.exit.is_some_and(is_disconnect))
        {
            saw_disconnect = true;
        }
        if reconnect::pending(&sess, 1) {
            saw_pending = true;
        }
        let text = pane_text(&mut sess, 1);
        if let Some(s) = sess.map.get(&1) {
            if s.exit.is_none() {
                let p = vtask::child_pid(s);
                if p > 0 && p != old_pid {
                    respawn_pid = Some(p);
                }
            }
        }
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        reconnect::pump(&st, &mut sess);
        ensure_sessions(&st, &mut sess);
        if !st.panes.contains_key(&1) || st.windows.is_empty() || ui.quitting {
            pane_kept = false;
            break;
        }
        if respawn_pid.is_some() && text.contains(&marker) {
            recovered = true;
            break;
        }
        sleep(FRAME_STEP);
    }

    // Snapshot the post-loop invariants before cleanup mutates `sess`.
    let pending_cleared = !reconnect::pending(&sess, 1);
    let pane_in_tree = st.panes.contains_key(&1) && !st.windows.is_empty();

    // Cleanup: drop every live pane child, then delete the server-side
    // zellij session (it deliberately survives the ssh drop).
    for id in sess.map.keys().copied().collect::<Vec<_>>() {
        terminate(&mut sess, id);
    }
    delete_zellij_session(&zsession);

    assert!(
        pane_kept && pane_in_tree,
        "close_exited must not close a disconnected remote pane"
    );
    assert!(
        saw_disconnect,
        "ssh exit was never observed as a disconnect"
    );
    assert!(
        saw_pending,
        "reconnect backoff window (pending badge) never observed"
    );
    assert!(
        respawn_pid.is_some_and(|p| p != old_pid),
        "no respawned ssh child observed (old {old_pid}, got {respawn_pid:?})"
    );
    assert!(recovered, "marker {marker} not restored after reattach");
    assert!(
        pending_cleared,
        "reconnecting badge never cleared after recovery"
    );
}
