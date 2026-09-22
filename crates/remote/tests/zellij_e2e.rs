//! Local-host e2e: ssh + bootstrap + zellij attach, via the real PTY path.
//! Run manually: cargo test -p remote --test zellij_e2e -- --ignored --nocapture
//! Requires: local sshd with key auth, zellij on PATH.

use std::time::{Duration, Instant};

#[test]
#[ignore = "needs local sshd + zellij"]
fn ssh_bootstrap_creates_zellij_session() {
    let session = format!("zt-e2e-{}", std::process::id());
    let target = remote::RemoteTarget {
        label: "e2e".into(),
        host: "localhost".into(),
        user: None,
        port: None,
        session_name: session.clone(),
    };
    let plan = remote::remote_plan(&target, remote::DEFAULT_PALETTE_HEX, true);
    let opts = vt_pane::SessionOpts::command(80, 24, plan.argv);
    let mut sess = vt_pane::task::spawn_session(&opts).expect("spawn ssh session");

    // The bootstrap config is chrome-free (flat theme, no pane frames,
    // cleared keybinds), so zellij never draws its own label. The honest
    // gate is: the loading screen cleared and the inner shell rendered,
    // then a typed marker must round-trip through zellij.
    let start = Instant::now();
    let mut rendered = String::new();
    while start.elapsed() < Duration::from_secs(30) {
        vt_pane::task::pump(&mut sess).ok();
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(f) = vt_pane::task::frame(&mut sess) {
            let text: String = f
                .cells
                .iter()
                .flat_map(|row| row.iter().map(|c| c.text.clone()))
                .collect::<String>()
                .trim()
                .to_string();
            if !text.is_empty()
                && !text.contains("Loading Zellij")
                && !text.contains("Querying terminal emulator")
            {
                rendered = text;
                break;
            }
        }
    }
    assert!(
        !rendered.is_empty(),
        "no content rendered from remote zellij"
    );

    // Round-trip proof: type a unique marker into the zellij pane and wait
    // for its echo.
    let marker = format!("ZTOK{}", std::process::id());
    vt_pane::task::paste(&mut sess, &format!("echo {marker}")).ok();
    vt_pane::task::send_key(&mut sess, |ev| {
        ev.set_action(libghostty_vt::key::Action::Press);
        ev.set_key(libghostty_vt::key::Key::Enter);
    })
    .ok();
    let start = Instant::now();
    let mut echoed = false;
    while start.elapsed() < Duration::from_secs(10) {
        vt_pane::task::pump(&mut sess).ok();
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(f) = vt_pane::task::frame(&mut sess) {
            let text: String = f
                .cells
                .iter()
                .flat_map(|row| row.iter().map(|c| c.text.clone()))
                .collect();
            if text.contains(&marker) {
                echoed = true;
                break;
            }
        }
    }
    assert!(echoed, "typed marker did not round-trip through zellij");

    // Session must exist on the "remote" (localhost).
    let out = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "localhost", "zellij list-sessions"])
        .output()
        .expect("ssh list");
    let list = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(list.contains(&session), "session missing in: {list}");

    // Bootstrap files exist.
    assert!(std::path::Path::new(&home().join(".cache/zt/config.kdl")).exists());

    // Cleanup: drop ssh pane (detach), then delete only this test's session
    // (`delete-all-sessions` would destroy other users' sessions).
    drop(sess);
    let _ = std::process::Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "localhost",
            "zellij",
            "delete-session",
            &session,
            "--force",
        ])
        .output();
}

fn home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/root"))
}

/// Concatenated cell text of a frame.
fn frame_text(f: &vt_pane::Frame) -> String {
    f.cells
        .iter()
        .flat_map(|row| row.iter().map(|c| c.text.clone()))
        .collect()
}

/// Wait until the zellij loading gate cleared and the inner shell
/// rendered (same gate as `ssh_bootstrap_creates_zellij_session`).
fn wait_past_gate(sess: &mut vt_pane::Session) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
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

/// Drain until the frame holds `needle`; returns the last text seen.
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

/// A network drop kills only the local ssh client: the zellij session
/// survives on the host, so spawning the SAME plan again (what the app's
/// reconnect pump does) must reattach with the session state intact.
#[test]
#[ignore = "needs local sshd + zellij"]
fn reattach_recovers_session_state() {
    let pid = std::process::id();
    let session = format!("zt-reattach-{pid}");
    let target = remote::RemoteTarget {
        label: "e2e".into(),
        host: "localhost".into(),
        user: None,
        port: None,
        session_name: session.clone(),
    };
    let plan = remote::remote_plan(&target, remote::DEFAULT_PALETTE_HEX, true);
    let opts = vt_pane::SessionOpts::command(80, 24, plan.argv);
    let mut first = match vt_pane::task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    assert!(wait_past_gate(&mut first), "first attach never rendered");

    // A marker only the SURVIVING session can still show after the drop.
    let marker = format!("ZREC{pid}");
    send_line(&mut first, &format!("echo {marker}"));
    let text = wait_text(&mut first, &marker, 10);
    assert!(
        text.contains(&marker),
        "marker never round-tripped: {text:?}"
    );

    // Simulate the network drop on our side: kill the ssh client. The
    // zellij session itself keeps running on the host.
    vt_pane::task::terminate(&mut first);
    std::thread::sleep(Duration::from_secs(1));

    // Reconnect = spawn the SAME plan (idempotent bootstrap reattaches
    // the remembered session via `zellij attach --create`).
    let mut second = match vt_pane::task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    assert!(wait_past_gate(&mut second), "reattach never rendered");
    let text = wait_text(&mut second, &marker, 10);
    assert!(
        text.contains(&marker),
        "reattached session lost its state: {text:?}"
    );

    // Cleanup: drop the ssh pane (detach), then delete only this test's
    // session, same as `ssh_bootstrap_creates_zellij_session`.
    vt_pane::task::terminate(&mut second);
    let _ = std::process::Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "localhost",
            "zellij",
            "delete-session",
            &session,
            "--force",
        ])
        .output();
}

#[test]
#[ignore = "needs local sshd"]
fn no_zellij_path_exits_42() {
    // Hide zellij from PATH; bootstrap must exit 42 (degrade marker).
    let target = remote::RemoteTarget {
        label: "e2e".into(),
        host: "localhost".into(),
        user: None,
        port: None,
        session_name: "zt-none".into(),
    };
    let boot = remote::bootstrap_command(&target, remote::DEFAULT_PALETTE_HEX);
    let out = std::process::Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "localhost",
            &format!("PATH=/nonexistent {boot}"),
        ])
        .output()
        .expect("ssh");
    assert_eq!(out.status.code(), Some(42), "expected exit 42");
}
