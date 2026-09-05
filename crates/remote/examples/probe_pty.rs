//! Debug: spawn the remote plan in a PTY, dump rendered rows.
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let session = format!("zt-probe-{}", std::process::id());
    let target = remote::RemoteTarget {
        label: "p".into(),
        host: "localhost".into(),
        user: None,
        port: None,
        session_name: session,
    };
    let plan = remote::remote_plan(&target, remote::DEFAULT_PALETTE_HEX, true);
    println!("argv: {:?}", plan.argv);
    let opts = vt_pane::SessionOpts::command(100, 28, plan.argv);
    let mut sess = vt_pane::task::spawn_session(&opts)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        vt_pane::task::pump(&mut sess).ok();
        std::thread::sleep(Duration::from_millis(300));
        if let Some(code) = sess.exit {
            println!("EXITED code={code}");
            break;
        }
    }
    let f = vt_pane::task::frame(&mut sess)?;
    for (i, row) in f.cells.iter().enumerate() {
        let line: String = row.iter().map(|c| c.text.as_str()).collect();
        println!("{i:02}|{line}");
    }
    Ok(())
}
