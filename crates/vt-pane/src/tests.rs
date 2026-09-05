//! Headless acceptance tests: feed VT data, assert on the plain-data grid.

use crate::task::{self, SessionOpts};
use crate::term::{snapshot_frame, Color};
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::Terminal;

fn frame_of(feeder: impl FnOnce(&mut Terminal<'static, 'static>)) -> crate::term::Frame {
    let mut term = Terminal::new(20, 4).expect("terminal");
    feeder(&mut term);
    let mut rs = RenderState::new().expect("render state");
    let mut ri = RowIterator::new().expect("row iterator");
    let mut ci = CellIterator::new().expect("cell iterator");
    snapshot_frame(&mut term, &mut rs, &mut ri, &mut ci).expect("frame")
}

fn row_text(frame: &crate::term::Frame, y: usize) -> String {
    frame.cells[y]
        .iter()
        .map(|c| c.text.as_str())
        .collect::<String>()
}

#[test]
fn plain_text_lands_in_grid() {
    let frame = frame_of(|t| t.vt_write(b"hello\r\nworld"));
    assert_eq!(row_text(&frame, 0), "hello");
    assert_eq!(row_text(&frame, 1), "world");
    assert_eq!(frame.cursor.x, 5);
    assert_eq!(frame.cursor.y, 1);
}

#[test]
fn sgr_colors_and_flags_resolve() {
    let frame = frame_of(|t| {
        t.vt_write(b"\x1b[1;32mgreen-bold\x1b[0m \x1b[38;2;255;128;0morange\x1b[0m");
    });
    assert_eq!(row_text(&frame, 0), "green-bold orange");
    let green = &frame.cells[0][0];
    assert!(green.bold);
    let g = green.fg.expect("palette green");
    assert!(g.g >= g.r && g.g > g.b, "expected greenish, got {g:?}");
    let orange = &frame.cells[0][11];
    assert_eq!(
        orange.fg,
        Some(Color {
            r: 255,
            g: 128,
            b: 0
        })
    );
    // Reset cell: no explicit color.
    let reset_space = &frame.cells[0][10];
    assert_eq!(reset_space.fg, None);
}

#[test]
fn inverse_video_swaps_rendering_hint() {
    let frame = frame_of(|t| t.vt_write(b"\x1b[7minv\x1b[27m"));
    assert!(frame.cells[0][0].inverse);
    assert!(!frame.cells[0][3].inverse);
}

#[test]
fn cursor_movement_and_clear() {
    let frame = frame_of(|t| {
        t.vt_write(b"abcdef");
        t.vt_write(b"\x1b[3D"); // left 3
        t.vt_write(b"XY"); // overwrite "de"
        t.vt_write(b"\x1b[2J"); // clear screen (cursor stays)
    });
    assert_eq!(row_text(&frame, 0), "");
    assert_eq!(frame.cursor.x, 5);
}

#[test]
fn ls_style_directory_listing_renders() {
    // Emulates `ls --color` output: colored names in columns.
    let frame = frame_of(|t| {
        t.vt_write(b"\x1b[01;34mcrates\x1b[0m  \x1b[32massets\x1b[0m\r\n");
        t.vt_write(b"\x1b[32mscripts\x1b[0m");
    });
    let first = row_text(&frame, 0);
    assert!(first.contains("crates"));
    assert!(first.contains("assets"));
    assert!(row_text(&frame, 1).contains("scripts"));
    assert!(frame.cells[0][0].bold, "dir entries are bold in ls --color");
}

// -------- pty integration --------

fn wait_output(sess: &mut crate::task::Session, millis: u64) {
    for _ in 0..(millis / 10) {
        task::pump(sess).expect("pump");
        if sess.exit.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn pty_roundtrip_printf() {
    let opts = SessionOpts::command(
        30,
        5,
        vec![
            "printf".to_string(),
            "\x1b[31mRED\x1b[0m ok\\nsecond-line".to_string(),
        ],
    );
    let mut sess = task::spawn_session(&opts).expect("session");
    for _ in 0..50 {
        wait_output(&mut sess, 100);
        if sess.exit.is_some() {
            break;
        }
    }
    task::pump(&mut sess).expect("pump");
    let frame = task::frame(&mut sess).expect("frame");
    let text: Vec<String> = (0..frame.rows as usize)
        .map(|y| row_text(&frame, y))
        .collect();
    let joined = text.join("|");
    assert!(joined.contains("RED"), "grid was: {joined:?}");
    assert!(joined.contains("second-line"), "grid was: {joined:?}");
    let red = frame.cells[0][0].fg.expect("palette red");
    assert!(
        red.r > red.g && red.r > red.b,
        "expected reddish, got {red:?}"
    );
}

#[test]
fn pty_echo_and_resize() {
    let opts = SessionOpts::command(20, 4, vec!["cat".to_string()]);
    let mut sess = task::spawn_session(&opts).expect("session");
    task::write(&sess, b"ping\n").expect("write");
    let mut found = false;
    for _ in 0..30 {
        wait_output(&mut sess, 100);
        let frame = task::frame(&mut sess).expect("frame");
        if row_text(&frame, 0).starts_with("ping") {
            found = true;
            break;
        }
    }
    assert!(found, "echo did not come back");
    task::resize(&mut sess, 40, 10, 8, 16).expect("resize");
    task::write(&sess, b"\x1b[6n").expect("cursor query");
    // After resize the terminal reports 40 columns (CPR comes back too).
    let mut resized = false;
    for _ in 0..30 {
        wait_output(&mut sess, 100);
        let frame = task::frame(&mut sess).expect("frame");
        if frame.cols == 40 {
            resized = true;
            break;
        }
    }
    assert!(resized, "resize did not apply");
}

#[test]
fn pty_exit_status_propagates() {
    let opts = SessionOpts::command(
        20,
        4,
        vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
    );
    let mut sess = task::spawn_session(&opts).expect("session");
    for _ in 0..50 {
        wait_output(&mut sess, 100);
        if sess.exit.is_some() {
            break;
        }
    }
    assert_eq!(sess.exit, Some(7));
}

#[test]
fn terminate_is_safe_on_dead_session() {
    // Spawn a child that exits immediately, wait for its exit status, then
    // terminate the (already dead) session: no panic, and the exit status
    // stays visible. Skips on pty-less CI.
    let opts = SessionOpts::command(20, 4, vec!["true".to_string()]);
    let mut sess = match task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            println!("skip: cannot spawn pty session: {e}");
            return;
        }
    };
    for _ in 0..50 {
        wait_output(&mut sess, 100);
        if sess.exit.is_some() {
            break;
        }
    }
    task::terminate(&mut sess);
    for _ in 0..10 {
        wait_output(&mut sess, 50);
        if sess.exit.is_some() {
            break;
        }
    }
    assert!(sess.exit.is_some(), "exit status should still be visible");
}

#[test]
fn terminate_kills_live_child_and_cleans_up() {
    // The core leak fix: a running child (that ignores nothing, like sleep)
    // must die via terminate; nothing may keep the pty master open.
    let opts = SessionOpts::command(20, 4, vec!["sleep".to_string(), "300".to_string()]);
    let mut sess = match task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    let pid = task::child_pid(&sess);
    assert!(pid > 0);
    task::terminate(&mut sess);
    // Reaped by the watchdog (SIGHUP exit or the SIGKILL escalation).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut dead = false;
    while std::time::Instant::now() < deadline {
        if matches!(crate::pty::pty_wait(pid, true), Ok(Some(_))) {
            dead = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(dead, "child {pid} survived terminate()");
}

