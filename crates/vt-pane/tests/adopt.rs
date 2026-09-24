//! Session migration integration: stop the reader, snapshot the terminal,
//! adopt the pty into a fresh Session, and prove screen + scrollback state
//! survive the hand-off on a REAL child (`cat`).

use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use vt_pane::task as vtask;
use vt_pane::{PtyHandle, Session, SessionOpts};

/// All visible cell text of a frame, rows joined by '\n'.
fn frame_text(sess: &mut Session) -> String {
    let frame = vtask::frame(sess).expect("frame");
    (0..frame.rows as usize)
        .map(|y| {
            frame.cells[y]
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pump in small steps until `pred(frame_text)` holds or the deadline hits.
fn pump_until(sess: &mut Session, millis: u64, pred: impl Fn(&str) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(millis);
    loop {
        vtask::pump(sess).expect("pump");
        if pred(&frame_text(sess)) {
            return true;
        }
        if Instant::now() >= deadline || sess.exit.is_some() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Poll a condition in 10ms steps up to `millis`.
fn poll_until(millis: u64, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    f()
}

/// One full migration cycle against a live `cat`:
///
/// 1. echo round-trip on the ORIGINAL session,
/// 2. overflow the 24-row screen with 60 lines (scrollback to save),
/// 3. dup the master (MUST precede stop_reader: `dup_master` refuses
///    once the reader has closed the master; the dup is independent of
///    that close, which is what keeps the child's pty alive across it),
/// 4. stop the reader, wait for it to close the master, drain, snapshot,
/// 5. adopt the dup'd fd + snapshot into a new Session,
/// 6. verify live I/O, child identity, and restored history,
/// 7. terminate both sessions (the second signal hits a dead pid: no-op).
#[test]
fn adopt_roundtrip_preserves_child_screen_and_scrollback() {
    let rows: u64 = 24;
    let opts = SessionOpts::command(80, rows as u16, vec!["cat".to_string()]);
    let mut orig = vtask::spawn_session(&opts).expect("spawn cat");

    // --- 1. live echo on the original session --------------------------
    vtask::write(&orig, b"pre-migrate\n").expect("write pre-migrate");
    assert!(
        pump_until(&mut orig, 2_000, |t| t.contains("pre-migrate")),
        "original session never echoed pre-migrate"
    );

    // --- 2. overflow the screen: 60 lines into 24 rows ------------------
    // Delivery must be confirmed BEFORE the reader is stopped: the stop
    // contract assumes a quiesced terminal, and once the reader exits the
    // channel disconnect sets `exit`, so no post-stop wait can help.
    let scroll: Vec<u8> = (1..=60u32)
        .flat_map(|i| format!("scroll-line-{i:04}\n").into_bytes())
        .collect();
    vtask::write(&orig, &scroll).expect("write scroll lines");
    assert!(
        pump_until(&mut orig, 2_000, |t| t.contains("scroll-line-0060")),
        "scroll lines never echoed back"
    );
    let pid = vtask::child_pid(&orig);
    assert!(pid > 0, "child pid unknown");

    // --- 3. dup the master BEFORE stopping the reader -------------------
    let dup = vtask::dup_master(&orig).expect("dup master before stop");
    let dup_raw = dup.as_raw_fd();
    // adopt_session takes ownership of the fd (its reader thread becomes
    // the sole closer): hand over the raw fd and forget the OwnedFd so
    // the destructor never double-closes it.
    std::mem::forget(dup);

    // --- 4. quiesce the original, then snapshot -------------------------
    vtask::stop_reader(&orig);
    assert!(
        poll_until(600, || vtask::reader_done(&orig)),
        "reader thread did not close the master within 600ms"
    );
    // The reader is gone: after this point dup_master/write refuse the fd.
    assert!(vtask::dup_master(&orig).is_err());
    assert!(vtask::write(&orig, b"late\n").is_ok(), "guarded write");
    // Drain whatever Output events were still queued behind the stop.
    vtask::pump(&mut orig).expect("final pump");
    let snap = vtask::snapshot(&mut orig)
        .expect("snapshot call")
        .expect("encodable snapshot");
    assert!(!snap.is_empty(), "snapshot encoded to zero bytes");

    // --- 5. adopt the dup'd pty + snapshot into a new Session -----------
    let handle = PtyHandle {
        master_fd: dup_raw,
        child_pid: pid,
    };
    let mut adopted = vtask::adopt_session(handle, Some(&snap), &opts).expect("adopt session");

    // --- 6. the adopted session is the SAME live child ------------------
    assert_eq!(vtask::child_pid(&adopted), pid);
    {
        let f = vtask::frame(&mut adopted).expect("adopted frame");
        assert_eq!((f.cols, f.rows), (80, rows as u16), "adopted geometry");
    }

    // Scrollback survived: history grew past the screen height.
    let (_, total, len) = vt_pane::viewport::geometry(&adopted).expect("scrollbar geometry");
    assert!(total > rows, "scrollback lost: total={total} rows={rows}");
    assert_eq!(len, rows, "visible viewport must stay one screen");

    // Live I/O still flows through the adopted fd.
    vtask::write(&adopted, b"post-adopt\n").expect("write post-adopt");
    assert!(
        pump_until(&mut adopted, 2_000, |t| t.contains("post-adopt")),
        "adopted session never echoed post-adopt"
    );

    // Jump to the top of history: BOTH the pre-migration marker and the
    // first overflow line must be there (screen content was restored,
    // not just the tail).
    vt_pane::viewport::scroll_top(&mut adopted);
    let top = frame_text(&mut adopted);
    assert!(
        top.contains("pre-migrate") && top.contains("scroll-line-0001"),
        "top of adopted scrollback missing early lines: {top:?}"
    );
    let (offset, _, _) = vt_pane::viewport::geometry(&adopted).expect("geometry after scroll_top");
    assert_eq!(offset, 0, "scroll_top did not pin to the history top");
    vt_pane::viewport::scroll_bottom(&mut adopted);
    assert!(
        vt_pane::viewport::pinned(&adopted),
        "scroll_bottom should re-pin"
    );

    // --- 7. cleanup ------------------------------------------------------
    // orig: reader already closed its fd; terminate only signals the pid.
    // adopted: same pid (dead by then, a no-op) + its reader closes the dup.
    vtask::terminate(&mut orig);
    vtask::terminate(&mut adopted);
}
