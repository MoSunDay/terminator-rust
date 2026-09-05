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

    // Pump for up to 15s or until zellij rendered something.
    let start = Instant::now();
    let mut rendered = String::new();
    while start.elapsed() < Duration::from_secs(15) {
        vt_pane::task::pump(&mut sess).ok();
        std::thread::sleep(Duration::from_millis(200));
        if let Ok(f) = vt_pane::task::frame(&mut sess) {
            let text: String = f
                .cells
                .iter()
                .flat_map(|row| row.iter().map(|c| c.text.clone()))
                .collect();
            if text.contains("ZELLIJ") || text.len() > 400 {
                rendered = text;
                break;
            }
        }
    }
    assert!(!rendered.is_empty(), "no content rendered from remote zellij");

    // Session must exist on the "remote" (localhost).
    let out = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "localhost", "zellij list-sessions"])
        .output()
        .expect("ssh list");
    let list = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(list.contains(&session), "session missing in: {list}");

    // Bootstrap files exist.
    assert!(std::path::Path::new(&home().join(".cache/zt/config.kdl")).exists());

    // Cleanup: drop ssh pane (detach), then delete session.
    drop(sess);
    let _ = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "localhost", "zellij delete-all-sessions --force"])
        .output();
}

fn home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/root"))
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
