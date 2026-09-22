//! Local-host e2e: REAL network interruption + zellij session recovery.
//! Run manually:
//!   cargo test -p remote --test net_drop_e2e -- --ignored --nocapture
//! Requires: local sshd with passwordless key auth on 127.0.0.1, zellij
//! on the remote PATH, and root (iptables + kill -9 of a per-connection
//! sshd child). The test process lives inside an sshd session arriving
//! on an EXTERNAL interface; the outage rule REJECTs tcp/22 on `lo` only
//! and never touches it. NEVER stop/restart sshd or kill its listener.
//! Drops covered: (1) server-side RST - kill -9 the per-connection sshd
//! child found by `ss -tnp` port pairing (client exits 255); (2) an
//! outage window where iptables REJECT-with-tcp-reset on lo/22 makes
//! every reconnect fail FAST (exit 255, far below ConnectTimeout=10).

use std::time::{Duration, Instant};

/// Serialize the two tests: both flip the global lo/22 reachability
/// (iptables) and share the sshd port, so they must not interleave.
fn net_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    match LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Concatenated cell text of a frame.
fn frame_text(f: &vt_pane::Frame) -> String {
    f.cells
        .iter()
        .flat_map(|row| row.iter().map(|c| c.text.clone()))
        .collect()
}

/// Wait until the zellij loading gate cleared and the inner shell
/// rendered (same gate as the zellij_e2e suite).
fn wait_gate(sess: &mut vt_pane::Session, secs: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        vt_pane::task::pump(sess).ok();
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(f) = vt_pane::task::frame(sess) {
            let text = frame_text(&f).trim().to_string();
            if !text.is_empty()
                && !text.contains("Loading Zellij")
                && !text.contains("Querying terminal emulator")
            {
                return true;
            }
        }
    }
    false
}

/// Poll until `needle` shows in the frame; returns the last frame text.
fn wait_text(sess: &mut vt_pane::Session, needle: &str, secs: u64) -> String {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(secs) {
        vt_pane::task::pump(sess).ok();
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(f) = vt_pane::task::frame(sess) {
            last = frame_text(&f);
            if last.contains(needle) {
                return last;
            }
        }
    }
    last
}

/// Type a line into the session and press Enter.
fn send_line(sess: &mut vt_pane::Session, line: &str) {
    vt_pane::task::paste(sess, line).ok();
    vt_pane::task::send_key(sess, |ev| {
        ev.set_action(libghostty_vt::key::Action::Press);
        ev.set_key(libghostty_vt::key::Key::Enter);
    })
    .ok();
}

/// Poll pump until the child exited; the observed exit code if any.
fn wait_exit(sess: &mut vt_pane::Session, secs: u64) -> Option<i32> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        vt_pane::task::pump(sess).ok();
        if sess.exit.is_some() {
            return sess.exit;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    sess.exit
}

/// First `pid=N` of an `ss -tnp` line (the owning process of a socket).
fn pid_of(line: &str) -> Option<i32> {
    let tail = line.split("pid=").nth(1)?;
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// (local, peer) TCP ports of an `ss -tnp` line, IPv4 or IPv6 spelling.
fn ports(line: &str) -> Option<(u16, u16)> {
    let f = line.split_whitespace().collect::<Vec<_>>();
    let port = |i: usize| -> Option<u16> { f.get(i)?.rsplit(':').next()?.parse().ok() };
    Some((port(3)?, port(4)?))
}

/// Pid of the server-side sshd child serving the pane's ssh connection:
/// find the client's socket by pid, take its local (ephemeral) port,
/// then find the :22 socket whose peer is exactly that port.
fn server_sshd_pid(client_pid: i32) -> Option<i32> {
    let out = std::process::Command::new("ss").arg("-tnp").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let client_port = text
        .lines()
        .find(|l| pid_of(l) == Some(client_pid))
        .and_then(|l| ports(l).map(|(local, _)| local))?;
    text.lines()
        .filter(|l| l.contains("\"sshd\""))
        .find_map(|l| match ports(l) {
            Some((22, peer)) if peer == client_port => pid_of(l),
            _ => None,
        })
}

/// Forcibly reset the connection server-side: kill -9 the per-connection
/// sshd child (NOT the listener). False when no such child was found.
fn kill_server_side(client_pid: i32) -> bool {
    match server_sshd_pid(client_pid) {
        Some(pid) if pid > 1 => std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        found => {
            eprintln!("skip: no server sshd for client {client_pid} (found {found:?})");
            false
        }
    }
}

/// The shared iptables rule spec (identical for -I and -D, so the delete
/// is the exact inverse of the insert and is idempotent).
fn rule_args() -> Vec<String> {
    const RULE: &str = "-i lo -p tcp --dport 22 -j REJECT --reject-with \
        tcp-reset -m comment --comment zt-netdrop-e2e";
    RULE.split_whitespace().map(String::from).collect()
}

/// Run `iptables <op> INPUT <rule...>`; None if the binary did not run.
/// Uses .output() so a no-op -D never leaks iptables stderr noise.
fn run_iptables(op: &str) -> Option<std::process::ExitStatus> {
    let mut args = vec![op.to_string(), "INPUT".to_string()];
    args.extend(rule_args());
    std::process::Command::new("iptables")
        .args(&args)
        .output()
        .map(|o| o.status)
        .ok()
}

/// Open the outage window: REJECT (tcp-reset) lo -> tcp/22. False (with
/// a skip note) when iptables refuses, e.g. not root.
fn block_lo22() -> bool {
    match run_iptables("-I") {
        Some(s) if s.success() => true,
        other => {
            eprintln!("skip: iptables -I failed ({other:?}) - not root?");
            false
        }
    }
}

/// Close the outage window; errors are ignored on purpose (deleting a
/// rule that is not there must stay a silent no-op).
fn unblock_lo22() {
    let _ = run_iptables("-D");
}

/// Delete only this test's zellij session on the remote host.
fn delete_session(session: &str) {
    let remote = format!("zellij delete-session {session} --force");
    let _ = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "127.0.0.1"])
        .arg(remote)
        .output();
}

/// The remote target every test attaches to: loopback sshd (v4, so the
/// plain iptables lo/22 rule matches).
fn target_for(session: &str) -> remote::RemoteTarget {
    remote::RemoteTarget {
        label: "e2e".into(),
        host: "127.0.0.1".into(),
        user: None,
        port: None,
        session_name: session.to_string(),
    }
}

/// A server-side RST (kill -9 of the per-connection sshd child serving
/// the pane) must surface as a disconnect-shaped exit (255/negative),
/// and respawning the SAME plan must reattach the still-running zellij
/// session with its on-screen state intact.
#[test]
#[ignore = "needs local root sshd + zellij"]
fn server_reset_yields_disconnect_exit_and_reattach_keeps_state() {
    let _guard = net_lock();
    unblock_lo22();
    let pid = std::process::id();
    let session = format!("zt-drop-{pid}");
    let plan = remote::remote_plan(&target_for(&session), remote::DEFAULT_PALETTE_HEX, true);
    let opts = vt_pane::SessionOpts::command(80, 24, plan.argv);
    let mut sess = match vt_pane::task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    assert!(wait_gate(&mut sess, 10), "attach never rendered");

    let marker = format!("ZDROP{pid}");
    send_line(&mut sess, &format!("echo {marker}"));
    let text = wait_text(&mut sess, &marker, 10);
    assert!(
        text.contains(&marker),
        "marker never round-tripped: {text:?}"
    );

    let old = vt_pane::task::child_pid(&sess);
    if !kill_server_side(old) {
        vt_pane::task::terminate(&mut sess);
        delete_session(&session);
        panic!("no server-side sshd found for pane child {old}");
    }
    let code = wait_exit(&mut sess, 10);
    eprintln!("after server RST: pane exit = {code:?}");
    assert!(
        code.is_some_and(remote::is_disconnect),
        "server RST must yield a disconnect exit, got {code:?}"
    );

    // Immediate reconnect on the same plan; the bootstrap reattaches.
    vt_pane::task::terminate(&mut sess);
    let mut second = match vt_pane::task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            delete_session(&session);
            return;
        }
    };
    assert!(wait_gate(&mut second, 15), "reattach never rendered");
    let text = wait_text(&mut second, &marker, 10);
    assert!(
        text.contains(&marker),
        "reattached session lost its state: {text:?}"
    );

    vt_pane::task::terminate(&mut second);
    delete_session(&session);
}

/// While an lo/22 outage window is open, reconnect attempts must fail
/// FAST with a disconnect exit (REJECT, not a ConnectTimeout hang), and
/// once the window closes the same plan recovers the surviving zellij
/// session. Nothing is asserted while the iptables rule is inserted (a
/// panic there would strand the rule).
#[test]
#[ignore = "needs local root sshd + zellij"]
fn outage_window_retries_fail_fast_then_recovers() {
    let _guard = net_lock();
    unblock_lo22();
    let pid = std::process::id();
    let session = format!("zt-out-{pid}");
    let plan = remote::remote_plan(&target_for(&session), remote::DEFAULT_PALETTE_HEX, true);
    let opts = vt_pane::SessionOpts::command(80, 24, plan.argv);
    let mut sess = match vt_pane::task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    assert!(wait_gate(&mut sess, 10), "attach never rendered");

    let marker = format!("ZOUT{pid}");
    send_line(&mut sess, &format!("echo {marker}"));
    let text = wait_text(&mut sess, &marker, 10);
    assert!(
        text.contains(&marker),
        "marker never round-tripped: {text:?}"
    );

    let old = vt_pane::task::child_pid(&sess);
    if !block_lo22() {
        vt_pane::task::terminate(&mut sess);
        delete_session(&session);
        return;
    }

    // --- outage window: collect data only, NEVER assert in here ---
    kill_server_side(old);
    let code0 = wait_exit(&mut sess, 8);
    eprintln!("in-window drop exit = {code0:?}");
    vt_pane::task::terminate(&mut sess);

    let mut attempts: Vec<(i32, f64)> = Vec::new();
    let mut consecutive = 0usize;
    let window = Instant::now();
    for _ in 0..3 {
        if window.elapsed() >= Duration::from_secs(25) {
            break;
        }
        let t0 = Instant::now();
        match vt_pane::task::spawn_session(&opts) {
            Ok(mut retry) => match wait_exit(&mut retry, 8) {
                Some(code) => {
                    consecutive += 1;
                    attempts.push((code, t0.elapsed().as_secs_f64()));
                    vt_pane::task::terminate(&mut retry);
                }
                None => {
                    // Connection unexpectedly outlived the window.
                    consecutive = 0;
                    vt_pane::task::terminate(&mut retry);
                }
            },
            Err(e) => {
                eprintln!("in-window spawn failed: {e}");
                consecutive += 1;
                attempts.push((remote::EXIT_SSH_FAIL, t0.elapsed().as_secs_f64()));
            }
        }
        if consecutive >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    // --- window closed: rule gone, safe to assert again ---
    unblock_lo22();
    eprintln!("in-window attempts: {attempts:?}");

    assert!(
        code0.is_some_and(remote::is_disconnect),
        "in-window drop must be a disconnect exit, got {code0:?}"
    );
    assert!(
        attempts.len() >= 2,
        "expected >= 2 failed retries in the window, got {attempts:?}"
    );
    for (i, (code, secs)) in attempts.iter().enumerate() {
        assert!(
            remote::is_disconnect(*code),
            "retry {i} exit {code} is not a disconnect"
        );
        assert!(
            *secs < 8.0,
            "retry {i} took {secs:.2}s - REJECT must fail fast, not hang on ConnectTimeout"
        );
    }

    // Recovery: same plan reattaches, the marker is still on screen.
    let mut back = match vt_pane::task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            delete_session(&session);
            return;
        }
    };
    assert!(
        wait_gate(&mut back, 15),
        "post-outage reattach never rendered"
    );
    let text = wait_text(&mut back, &marker, 10);
    assert!(
        text.contains(&marker),
        "recovered session lost its state: {text:?}"
    );
    vt_pane::task::terminate(&mut back);

    // The zellij session survived the whole outage on the remote host.
    let argv = [
        "-o",
        "BatchMode=yes",
        "127.0.0.1",
        "zellij",
        "list-sessions",
    ];
    let out = std::process::Command::new("ssh")
        .args(argv)
        .output()
        .expect("ssh list");
    let list = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(list.contains(&session), "session missing in: {list}");

    delete_session(&session);
}
