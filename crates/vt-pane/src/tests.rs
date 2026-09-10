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

#[test]
fn cjk_wide_cells_flagged_with_spacer_tails() {
    // "\u{6C49}a\u{5B57}" = Han, 'a', Han: each Han cell is wide and
    // followed by an empty spacer tail cell.
    let frame = frame_of(|t| t.vt_write("\u{6C49}a\u{5B57}".as_bytes()));
    let row = &frame.cells[0];
    assert_eq!(row[0].text, "\u{6C49}");
    assert!(row[0].wide);
    assert_eq!(row[1].text, "");
    assert!(!row[1].wide);
    assert_eq!(row[2].text, "a");
    assert!(!row[2].wide);
    assert_eq!(row[3].text, "\u{5B57}");
    assert!(row[3].wide);
    assert_eq!(row[4].text, "");
    assert!(!row[4].wide);
    assert_eq!(frame.cursor.x, 5, "wide cells advance the cursor by two");
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
    for _ in 0..100 {
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

#[test]
fn write_after_reader_closed_fd_is_ok() {
    // The reader thread is the sole closer of the master fd. Once it has
    // closed the fd (poll timeout 200ms after terminate), a UI-thread
    // write must be a guarded no-op Ok(()) instead of Err(EBADF) -- the
    // fd number may already belong to a different pane's pty.
    let opts = SessionOpts::command(20, 4, vec!["sleep".to_string(), "300".to_string()]);
    let mut sess = match task::spawn_session(&opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    task::terminate(&mut sess);
    // The reader polls with a 200ms timeout; give it room to close the fd
    // and publish the closed flag.
    std::thread::sleep(std::time::Duration::from_millis(400));
    match task::write(&sess, b"x") {
        Ok(()) => {}
        Err(e) => panic!("write after fd close must be Ok, got {e}"),
    }
}

// ---------------------------------------------------------------------------
// Mouse reporting, wheel routing and selection gestures
// ---------------------------------------------------------------------------

mod mouse_tests {
    use crate::mouse::{self as vmouse, WheelRoute};
    use crate::task::{self, SessionOpts};
    use libghostty_vt::key::Mods as GMods;
    use libghostty_vt::mouse;

    /// A session whose terminal modes we drive directly via vt_write
    /// (child exits instantly; encode paths never need the pty).
    fn session() -> Option<task::Session> {
        let opts = SessionOpts::command(80, 24, vec!["true".to_string()]);
        match task::spawn_session(&opts) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("skip: {e}");
                None
            }
        }
    }

    #[test]
    fn sgr_press_release_bytes() {
        let Some(mut s) = session() else { return };
        s.term.vt_write(b"\x1b[?1000;1006h");
        assert!(vmouse::is_mouse_tracking(&s));
        // Surface px (30, 112) with 8x16 nominal cells -> grid (3, 7) ->
        // SGR 1-based (col 4, row 8).
        let press = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Press,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            112.0,
            false,
        )
        .expect("encode");
        assert_eq!(press, b"\x1b[<0;4;8M");
        let release = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Release,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            112.0,
            true,
        )
        .expect("encode");
        assert_eq!(release, b"\x1b[<0;4;8m");
    }

    #[test]
    fn normal_mode_reports_no_motion() {
        let Some(mut s) = session() else { return };
        s.term.vt_write(b"\x1b[?1000;1006h");
        // Mode 1000 (press/release only): a motion event encodes to nothing.
        let out = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Motion,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            112.0,
            true,
        )
        .expect("encode");
        assert!(out.is_empty());
    }

    #[test]
    fn button_motion_mode_dedups_per_cell() {
        let Some(mut s) = session() else { return };
        s.term.vt_write(b"\x1b[?1002;1006h");
        let a = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Motion,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            112.0,
            true,
        )
        .expect("encode");
        assert_eq!(a, b"\x1b[<32;4;8M");
        // Same cell again: deduplicated to nothing.
        let dup = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Motion,
            Some(mouse::Button::Left),
            GMods::empty(),
            31.0, // same 8x16 cell as (30, 112)
            118.0,
            true,
        )
        .expect("encode");
        assert!(dup.is_empty());
        // A new cell encodes again.
        let moved = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Motion,
            Some(mouse::Button::Left),
            GMods::empty(),
            40.0,
            112.0,
            true,
        )
        .expect("encode");
        assert_eq!(moved, b"\x1b[<32;6;8M");
    }

    #[test]
    fn right_button_drag_reports_its_button() {
        // 1002 (button-event mouse tracking) + SGR: a right-button drag
        // reports button 2 for press/motion/release (motion = 32 + 2).
        let Some(mut s) = session() else { return };
        s.term.vt_write(b"\x1b[?1002;1006h");
        assert!(vmouse::is_mouse_tracking(&s));
        let press = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Press,
            Some(mouse::Button::Right),
            GMods::empty(),
            30.0,
            112.0,
            false,
        )
        .expect("encode");
        assert_eq!(press, b"\x1b[<2;4;8M");
        // Motion in a different cell (px (38,128) -> grid (4,8) -> col 5
        // row 9) so per-cell dedup does not swallow it.
        let drag = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Motion,
            Some(mouse::Button::Right),
            GMods::empty(),
            38.0,
            128.0,
            true,
        )
        .expect("encode");
        assert_eq!(drag, b"\x1b[<34;5;9M");
        let release = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Release,
            Some(mouse::Button::Right),
            GMods::empty(),
            30.0,
            112.0,
            true,
        )
        .expect("encode");
        assert_eq!(release, b"\x1b[<2;4;8m");
    }

    #[test]
    fn tracking_format_change_resyncs_encoder() {
        // 1002 alone: legacy bytes for a Left press at grid (3,7).
        let Some(mut s) = session() else { return };
        s.term.vt_write(b"\x1b[?1002h");
        let legacy = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Press,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            112.0,
            false,
        )
        .expect("encode");
        assert_eq!(legacy, b"\x1b[M $(");
        // SGR enabled while tracking stays on: the encoder must re-sync
        // and switch format (1-based col 4, row 8).
        s.term.vt_write(b"\x1b[?1006h");
        let sgr = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Press,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            112.0,
            false,
        )
        .expect("encode");
        assert_eq!(sgr, b"\x1b[<0;4;8M");
    }

    #[test]
    fn tracking_kind_change_resyncs_encoder() {
        // 1000: no motion reporting; then 1002 while SGR stays on: motion
        // must start encoding.
        let Some(mut s) = session() else { return };
        s.term.vt_write(b"\x1b[?1000;1006h");
        let none = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Motion,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            112.0,
            true,
        )
        .expect("encode");
        assert!(none.is_empty());
        s.term.vt_write(b"\x1b[?1002h");
        let motion = vmouse::encode_mouse(
            &mut s,
            mouse::Action::Motion,
            Some(mouse::Button::Left),
            GMods::empty(),
            30.0,
            128.0,
            true,
        )
        .expect("encode");
        assert_eq!(motion, b"\x1b[<32;4;9M");
    }

    #[test]
    fn wheel_routing_rules() {
        // App grabs the mouse -> report, unless Shift overrides.
        assert_eq!(vmouse::wheel_route(true, false, false), WheelRoute::Report);
        assert_eq!(vmouse::wheel_route(true, true, false), WheelRoute::Viewport);
        // Full-screen apps without mouse reporting -> arrow keys.
        assert_eq!(vmouse::wheel_route(false, false, true), WheelRoute::Arrows);
        // Plain shell -> scrollback.
        assert_eq!(
            vmouse::wheel_route(false, false, false),
            WheelRoute::Viewport
        );
        assert_eq!(vmouse::wheel_route(false, true, true), WheelRoute::Viewport);
    }

    #[test]
    fn wheel_helpers_direction_and_scale() {
        assert_eq!(
            vmouse::wheel_arrows(1.0),
            Some((libghostty_vt::key::Key::ArrowUp, vmouse::WHEEL_STEP_LINES))
        );
        assert_eq!(
            vmouse::wheel_arrows(-2.0),
            Some((
                libghostty_vt::key::Key::ArrowDown,
                2 * vmouse::WHEEL_STEP_LINES
            ))
        );
        assert_eq!(vmouse::wheel_arrows(0.0), None);
        // Up scrolls into history (negative viewport delta).
        assert_eq!(
            vmouse::wheel_delta(1.0),
            Some(-(vmouse::WHEEL_STEP_LINES as isize))
        );
        assert_eq!(
            vmouse::wheel_delta(-1.0),
            Some(vmouse::WHEEL_STEP_LINES as isize)
        );
    }

    #[test]
    fn alt_screen_wheel_sends_arrow_keys() {
        let Some(mut s) = session() else { return };
        s.term.vt_write(b"\x1b[?1049h");
        assert!(vmouse::alt_screen(&s));
        // The child (already exited) cannot consume the bytes, but the
        // guarded write path must stay silent-successful.
        vmouse::send_wheel(&mut s, 1.0, GMods::empty(), 10.0, 10.0, false).expect("wheel");
    }

    #[test]
    fn selection_drag_and_copy_text() {
        let opts = SessionOpts::command(
            40,
            5,
            vec!["printf".to_string(), "hello mouse world".to_string()],
        );
        let mut s = match task::spawn_session(&opts) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        for _ in 0..100 {
            let _ = task::pump(&mut s);
            if let Ok(f) = task::frame(&mut s) {
                if f.cells[0].iter().any(|c| c.text == "h") {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Drag from col 6 to col 11 (8x16 nominal cells) -> "mouse".
        vmouse::select_press(&mut s, 6.0 * 8.0, 0.0).expect("press");
        vmouse::select_drag(&mut s, 11.0 * 8.0, 0.0, false).expect("drag");
        vmouse::select_release(&mut s, 11.0 * 8.0, 0.0).expect("release");
        let text = vmouse::selection_text(&mut s).expect("text");
        assert!(text.contains("mouse"), "selection: {text:?}");
        assert!(!text.contains("hello"), "selection: {text:?}");
        assert!(!text.contains("world"), "selection: {text:?}");
        // Rendered cells inside the selection are flagged.
        let f = task::frame(&mut s).expect("frame");
        let sel: Vec<bool> = f.cells[0].iter().map(|c| c.selected).collect();
        assert!(sel[6..11].iter().any(|v| *v), "flags: {sel:?}");
        assert!(!sel[..3].iter().any(|v| *v), "flags: {sel:?}");
        // A fresh press clears the selection again.
        vmouse::select_press(&mut s, 20.0 * 8.0, 0.0).expect("press");
        assert_eq!(vmouse::selection_text(&mut s).expect("text"), "");
    }

    #[test]
    fn double_click_selects_word() {
        let opts = SessionOpts::command(
            40,
            5,
            vec!["printf".to_string(), "alpha beta gamma".to_string()],
        );
        let mut s = match task::spawn_session(&opts) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        for _ in 0..100 {
            let _ = task::pump(&mut s);
            if let Ok(f) = task::frame(&mut s) {
                if f.cells[0].iter().any(|c| c.text == "a") {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Two presses inside the repeat interval -> word selection.
        let (x, y) = (7.0 * 8.0, 0.0); // middle of "beta"
        vmouse::select_press(&mut s, x, y).expect("press");
        vmouse::select_press(&mut s, x, y).expect("press 2");
        vmouse::select_release(&mut s, x, y).expect("release");
        let text = vmouse::selection_text(&mut s).expect("text");
        assert_eq!(text.trim(), "beta", "word selection: {text:?}");
    }
}

#[cfg(test)]
mod deadzone_tests {
    use crate::mouse as vmouse;
    use crate::task::{self, SessionOpts};

    fn session(cols: u16, rows: u16) -> Option<task::Session> {
        match task::spawn_session(&SessionOpts::command(cols, rows, vec!["true".into()])) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("skip: {e}");
                None
            }
        }
    }

    #[test]
    fn drag_into_dead_zone_below_grid_selects_last_row() {
        // 40x5 grid, 8x16 cells -> 128px of grid, 200px of "pane".
        let Some(mut s) = session(40, 5) else { return };
        s.term.vt_write(b"row0 row1 row2 row3 row4");
        // Press in row 0, release far below the grid (dead space): the
        // gesture must clamp to the last row instead of dropping.
        vmouse::select_press(&mut s, 4.0, 4.0).expect("press");
        vmouse::select_drag(&mut s, 60.0, 190.0, false).expect("drag");
        vmouse::select_release(&mut s, 60.0, 190.0).expect("release");
        let text = vmouse::selection_text(&mut s).expect("text");
        assert!(text.contains("row4"), "clamped selection: {text:?}");
    }

    #[test]
    fn press_beyond_grid_reports_last_cell() {
        let Some(mut s) = session(40, 5) else { return };
        s.term.vt_write(b"\x1b[?1000;1006h");
        let out = vmouse::encode_mouse(
            &mut s,
            libghostty_vt::mouse::Action::Press,
            Some(libghostty_vt::mouse::Button::Left),
            libghostty_vt::key::Mods::empty(),
            500.0,
            190.0,
            false,
        )
        .expect("encode");
        // Clamped to col 40, row 5 (1-based) instead of dropped.
        assert_eq!(out, b"\x1b[<0;40;5M");
    }
}
