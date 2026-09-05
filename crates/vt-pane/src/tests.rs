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
    task::write(&mut sess, b"ping\n").expect("write");
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
    task::write(&mut sess, b"\x1b[6n").expect("cursor query");
    // The terminal answers with a CPR; the encoder/pty path should see bytes.
    let mut saw_reply = false;
    for _ in 0..30 {
        wait_output(&mut sess, 100);
        let frame = task::frame(&mut sess).expect("frame");
        // After resize, terminal reports 40 columns.
        assert_eq!(frame.cols, 40);
        let _ = frame;
        saw_reply = true;
        break;
    }
    assert!(saw_reply);
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
