//! Snapshot tests: dirty-row skipping, the two independent dirty layers and
//! the trimmed per-cell reader, cross-checked against a pre-trimming copy.

use super::*;
use crate::task::{self, SessionOpts};
use libghostty_vt::render::{CellIterator, RowIterator};

type Iterators = (
    RenderState<'static>,
    RowIterator<'static>,
    CellIterator<'static>,
);

fn new_iterators() -> Iterators {
    let (rs, ri, ci) = (RenderState::new(), RowIterator::new(), CellIterator::new());
    (rs.expect("render"), ri.expect("rows"), ci.expect("cells"))
}

fn snapshot_into(
    term: &mut Terminal<'static, 'static>,
    it: &mut Iterators,
    frame: &mut Frame,
    force_full: bool,
) {
    snapshot_frame_into(term, &mut it.0, &mut it.1, &mut it.2, frame, force_full)
        .expect("snapshot");
}

fn fresh_frame(bytes: &[u8]) -> Frame {
    let mut term = Terminal::new(8, 3).expect("terminal");
    term.vt_write(bytes);
    let mut iters = new_iterators();
    let mut frame = Frame::default();
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    frame
}

fn row_text(f: &Frame, y: usize) -> String {
    f.cells[y].iter().map(|c| c.text.as_str()).collect()
}

fn all_text(frame: &Frame) -> String {
    frame
        .cells
        .iter()
        .flatten()
        .map(|c| c.text.as_str())
        .collect()
}

#[test]
fn snapshot_distinguishes_pending_wrap_from_a_cursor_in_the_last_column() {
    for text in ["abcdefgh", "汉字汉字"] {
        let before = fresh_frame(text.as_bytes());
        assert!(before.cursor.pending_wrap);
        assert!(before.cursor.autowrap);
        let after = fresh_frame(format!("{text}你").as_bytes());
        assert_eq!((after.cursor.x, after.cursor.y), (2, 1));
        assert!(!after.cursor.pending_wrap);
    }
    let positioned = fresh_frame(b"\x1b[1;8H");
    assert_eq!(positioned.cursor.x, 7);
    assert!(!positioned.cursor.pending_wrap);
    let no_wrap = fresh_frame(b"\x1b[?7labcdefgh");
    assert!(!no_wrap.cursor.autowrap);
    let overwritten = fresh_frame(b"\x1b[?7labcdefghi");
    assert_eq!((overwritten.cursor.x, overwritten.cursor.y), (7, 0));
    assert_eq!(overwritten.cells[0][7].text, "i");
}

// -------- reference reader: the pre-trimming semantics oracle --------

/// One row through the pre-trimming reader (five FFI calls per cell).
fn reference_cells(
    row: &RowIteration<'static, '_>,
    cells: &mut CellIterator<'static>,
) -> Vec<CellData> {
    let mut out = Vec::new();
    let mut iter = cells.update(row).expect("cells");
    while let Some(cell) = iter.next() {
        let mut text = String::new();
        let _ = cell.graphemes_utf8(&mut text);
        let mut data = CellData {
            text,
            ..CellData::default()
        };
        data.fg = cell.fg_color().ok().flatten().map(Color::from);
        data.bg = cell.bg_color().ok().flatten().map(Color::from);
        data.wide = data
            .text
            .chars()
            .next()
            .is_some_and(|c| c.width().unwrap_or(1) > 1);
        data.selected = cell.is_selected().unwrap_or(false);
        if cell.has_styling().unwrap_or(false) {
            if let Ok(style) = cell.style() {
                data.bold = style.bold;
                data.italic = style.italic;
                data.faint = style.faint;
                data.blink = style.blink;
                data.invisible = style.invisible;
                data.inverse = style.inverse;
                data.underline = !matches!(style.underline, libghostty_vt::style::Underline::None);
                data.strikethrough = style.strikethrough;
            }
        }
        out.push(data);
    }
    out
}

/// Full grid through the reference reader, padded like the snapshot path
/// so a comparison isolates per-cell semantics, not shape.
fn reference_frame(
    term: &mut Terminal<'static, 'static>,
    cols: u16,
    rows: u16,
) -> Vec<Vec<CellData>> {
    let mut iters = new_iterators();
    let snapshot = iters.0.update(term).expect("update");
    let mut out = Vec::new();
    let mut iter = iters.1.update(&snapshot).expect("rows");
    while let Some(row) = iter.next() {
        let mut cells = reference_cells(row, &mut iters.2);
        cells.resize_with(cols as usize, CellData::default);
        out.push(cells);
    }
    out.resize_with(rows as usize, || vec![CellData::default(); cols as usize]);
    out
}

/// Payloads whose per-cell fields must survive the trimming unchanged.
const TRIM_PAYLOADS: [&[u8]; 9] = [
    b"hello\r\nworld \x1b[4munder\x1b[0m",
    b"\x1b[1;32mgreen-bold\x1b[0m \x1b[38;2;255;128;0morange\x1b[0m",
    b"\x1b[5mblinky\x1b[0m\x1b[2mfaint\x1b[0m\x1b[8mhidden\x1b[0m\x1b[9mstrike",
    "\x1b[7minverse 汉字 🤖".as_bytes(),
    b"\x1b[41m\x1b[H\x1b[2J",
    b"\x1b[41m\x1b[2;3H\x1b[K",
    b"\x1b[48;5;208m\x1b[3;1H\x1b[4X",
    b"\x1b[7m\x1b[4;1H\x1b[5X",
    b"\x1b[3;2H\x1b[3mitalic\x1b[0m\x1b[53moverline",
];

#[test]
fn trimmed_cell_reading_matches_the_reference_reader() {
    for payload in TRIM_PAYLOADS {
        let mut term = Terminal::new(20, 5).expect("terminal");
        term.vt_write(payload);
        let mut iters = new_iterators();
        let mut frame = Frame::default();
        snapshot_into(&mut term, &mut iters, &mut frame, false);
        let want = reference_frame(&mut term, 20, 5);
        assert_eq!(frame.cells, want, "payload: {payload:?}");
    }
}

// -------- in-place / dirty-row updates --------

/// A 10x6 terminal with a distinct filler on every row, plus the frame and
/// the long-lived iterators a session would keep. Rows 2-4 then carry
/// SENT* markers the way a caller scribbles on a cached frame; no write in
/// the tests below touches those rows, so any rebuild is observable.
fn partial_setup() -> (Terminal<'static, 'static>, Frame, Iterators) {
    let mut term = Terminal::new(10, 6).expect("terminal");
    // Address every row explicitly: a trailing CRLF on the last row would
    // scroll the screen and shift the whole grid up by one.
    for y in 0..6 {
        term.vt_write(format!("\x1b[{};1H{}", y + 1, format!("{y}").repeat(10)).as_bytes());
    }
    term.vt_write(b"\x1b[6;1H");
    let mut iters = new_iterators();
    let mut frame = Frame::default();
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    for y in 0..6 {
        assert_eq!(
            row_text(&frame, y),
            format!("{}{}", y, y.to_string().repeat(9))
        );
    }
    // Scribble on the three rows below the write region; only a rebuild
    // can wipe these (the entries are not valid terminal content).
    for (y, mark) in [(2usize, "SENT2"), (3, "SENT3"), (4, "SENT4")] {
        frame.cells[y][0].text = mark.into();
        frame.cells[y][0].bold = true;
        frame.cells[y][1].text = "ZZZ".into();
    }
    (term, frame, iters)
}

#[test]
fn clean_frame_does_no_row_work() {
    let (mut term, mut frame, mut iters) = partial_setup();
    // Nothing written since the last snapshot: the engine reports a clean
    // frame, so EVERY cached cell (markers and all) must survive verbatim.
    for (y, row) in frame.cells.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = CellData {
                text: format!("{x}.{y}"),
                bold: true,
                ..CellData::default()
            };
        }
    }
    let before = frame.clone();
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    assert_eq!(frame, before, "a clean frame did row work");
}

#[test]
fn partial_update_skips_clean_rows_and_keeps_their_cells() {
    let (mut term, mut frame, mut iters) = partial_setup();
    // Two writes that only touch rows 0/1 (plus the cursor's old row 5):
    // row 3 is provably clean, so its cached cells must survive verbatim.
    term.vt_write(b"\x1b[1;1HX\x1b[2;1HY");
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    assert_eq!(frame.cells[0][0].text, "X", "dirty row not updated");
    assert_eq!(frame.cells[1][0].text, "Y", "dirty row not updated");
    assert_eq!(row_text(&frame, 5), "5555555555");
    for (y, mark) in [(2usize, "SENT2"), (3, "SENT3"), (4, "SENT4")] {
        assert_eq!(frame.cells[y][0].text, mark, "clean row {y} was rebuilt");
        assert_eq!(frame.cells[y][1].text, "ZZZ", "clean row {y} was rebuilt");
        assert!(
            frame.cells[y][0].bold,
            "clean row {y} lost its cached flags"
        );
    }
    assert_eq!(row_text(&frame, 3), "SENT3ZZZ33333333");
}

#[test]
fn engine_full_dirty_rebuilds_every_row() {
    let (mut term, mut frame, mut iters) = partial_setup();
    // A full-screen erase is an engine `Full`: no rows are flagged
    // individually, so the patch path must fall back to a full rebuild.
    term.vt_write(b"\x1b[41m\x1b[H\x1b[2J");
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    assert!(all_text(&frame).is_empty(), "full clear left stale text");
    assert_eq!(frame.cells, reference_frame(&mut term, 10, 6));
    let bg = frame.cells[3][3].bg.expect("BCE background");
    assert!(
        bg.r > bg.g && bg.r > bg.b,
        "erased cells lost their bg: {bg:?}"
    );
}

#[test]
fn forced_full_rebuild_replaces_every_cached_cell() {
    let (mut term, mut frame, mut iters) = partial_setup();
    term.vt_write(b"\x1b[1;1HX");
    snapshot_into(&mut term, &mut iters, &mut frame, true);
    assert_eq!(frame.cells[0][0].text, "X");
    for y in 2..5 {
        assert_eq!(frame.cells[y][0].text, format!("{y}"), "sentinel survived");
        assert!(!frame.cells[y][0].bold);
    }
}

#[test]
fn reshape_on_resize_rebuilds_the_grid_at_the_new_geometry() {
    let (mut term, mut frame, mut iters) = partial_setup();
    term.resize(14, 3, 9, 18).expect("resize");
    term.vt_write(b"\x1b[1;14HZ");
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    assert_eq!((frame.cols, frame.rows), (14, 3));
    assert_eq!(frame.cells.len(), 3);
    assert!(frame.cells.iter().all(|r| r.len() == 14));
    assert!(frame
        .cells
        .iter()
        .flatten()
        .all(|c| !c.text.starts_with("SENT")));
    assert_eq!(frame.cells[0][13].text, "Z");
    // The reflowed grid must equal a fresh full read of the terminal.
    assert_eq!(frame.cells, reference_frame(&mut term, 14, 3));
}

#[test]
fn has_blink_follows_partial_updates() {
    let (mut term, mut frame, mut iters) = partial_setup();
    assert!(!frame.has_blink);
    term.vt_write(b"\x1b[2;5H\x1b[5mB");
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    assert!(frame.cells[1][4].blink);
    assert!(frame.has_blink, "a blinking row must raise the flag");
    // Repaint that row without blink: the flag has to drop again.
    term.vt_write(b"\x1b[2;1H\x1b[25m\x1b[K");
    snapshot_into(&mut term, &mut iters, &mut frame, false);
    assert!(
        !all_text(&frame).contains('B'),
        "the blink cell survived the erase"
    );
    assert!(!frame.has_blink, "stale blink flag kept");
}

// -------- selection: the row-local range decode --------

/// Cross-checks the row-local selection range used by the snapshot against
/// the engine's own per-cell answer for a real drag (both directions).
#[test]
fn selection_flags_match_the_per_cell_answer() {
    use crate::mouse as vmouse;

    let mut sess = match task::spawn_session(&SessionOpts::command(24, 6, vec!["cat".into()])) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    for y in 0..6 {
        sess.term
            .vt_write(format!("row{y} 0123456789abcdef\r\n").as_bytes());
    }
    for (fx, fy, tx, ty) in [(2usize, 0usize, 5usize, 2usize), (5, 2, 2, 0)] {
        let (fx, fy, tx, ty) = (
            fx as f32 * 8.0,
            fy as f32 * 16.0,
            tx as f32 * 8.0,
            ty as f32 * 16.0,
        );
        vmouse::select_press(&mut sess, fx, fy).expect("press");
        // both drag directions, so a reversed range is covered too
        vmouse::select_drag(&mut sess, tx, ty, false).expect("drag");
        vmouse::select_release(&mut sess, tx, ty).expect("release");

        let mut iters = new_iterators();
        let mut frame = Frame::default();
        snapshot_into(&mut sess.term, &mut iters, &mut frame, true);

        let mut oracle = new_iterators();
        let snapshot = oracle.0.update(&sess.term).expect("update");
        let mut rows = oracle.1.update(&snapshot).expect("rows");
        let (mut y, mut selected) = (0usize, 0usize);
        while let Some(row) = rows.next() {
            let mut cells = oracle.2.update(row).expect("cells");
            let mut x = 0usize;
            while let Some(cell) = cells.next() {
                let want = cell.is_selected().expect("is_selected");
                assert_eq!(frame.cells[y][x].selected, want, "cell ({x},{y})");
                selected += usize::from(want);
                x += 1;
            }
            y += 1;
        }
        assert_eq!(y, frame.rows as usize);
        assert!(selected > 0, "the drag selected nothing");
    }
}

// -------- session-level cache invalidation --------

fn spawn_cat(cols: u16, rows: u16) -> Option<task::Session> {
    match task::spawn_session(&SessionOpts::command(cols, rows, vec!["cat".into()])) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("skip: {e}");
            None
        }
    }
}

/// Pump until `needle` shows up through the UNCACHED frame path.
fn wait_text(sess: &mut task::Session, needle: &str) -> Option<Frame> {
    for _ in 0..300 {
        task::pump(sess).expect("pump");
        let frame = task::frame(sess).expect("frame");
        if all_text(&frame).contains(needle) {
            return Some(frame);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

#[test]
fn frame_cached_patches_rows_after_output() {
    let Some(mut sess) = spawn_cat(30, 8) else {
        return;
    };
    task::write(&sess, b"first-line\n").expect("write");
    assert!(wait_text(&mut sess, "first-line").is_some(), "no echo");

    // Scribble on a row far below the cursor: output only invalidates via
    // `pump` (the row-patching path), so this must survive.
    let mut frame = task::frame_cached(&mut sess).clone();
    frame.cells[7][0].text = "SENTINEL".into();
    sess.cache = frame;
    sess.cache_valid = true;

    task::write(&sess, b"second-line\n").expect("write");
    let mut text = String::new();
    for _ in 0..300 {
        task::pump(&mut sess).expect("pump");
        let frame = task::frame_cached(&mut sess);
        text = all_text(frame);
        if text.contains("second-line") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        text.contains("second-line"),
        "output never patched in: {text:?}"
    );
    assert!(text.contains("first-line"), "lost the first line: {text:?}");
    assert!(
        text.contains("SENTINEL"),
        "a clean row was rebuilt: {text:?}"
    );

    // A selection/theme-style (full) invalidation must rebuild every row,
    // so the scribble cannot survive it.
    task::invalidate_frame(&mut sess);
    let frame = task::frame_cached(&mut sess).clone();
    assert!(
        !all_text(&frame).contains("SENTINEL"),
        "full invalidation kept a row"
    );
    assert!(all_text(&frame).contains("second-line"));
}

#[test]
fn uncached_frame_fully_invalidates_the_row_cache() {
    let Some(mut sess) = spawn_cat(30, 8) else {
        return;
    };
    task::write(&sess, b"alpha\n").expect("write");
    assert!(wait_text(&mut sess, "alpha").is_some(), "no echo");

    // Cached frame with a scribble on the last row.
    let mut frame = task::frame_cached(&mut sess).clone();
    frame.cells[7][0].text = "SENTINEL".into();
    sess.cache = frame;
    sess.cache_valid = true;

    // New output, then the UNCACHED pass: it consumes the engine's dirty
    // layers, so the cached grid must be marked for a FULL rebuild or the
    // next cached frame would silently miss `bravo`.
    task::write(&sess, b"bravo\n").expect("write");
    for _ in 0..300 {
        task::pump(&mut sess).expect("pump");
        let fresh = task::frame(&mut sess).expect("frame");
        if !all_text(&fresh).contains("bravo") {
            std::thread::sleep(std::time::Duration::from_millis(10));
            continue;
        }
        let cached = task::frame_cached(&mut sess).clone();
        let text = all_text(&cached);
        assert!(text.contains("bravo"), "cached frame went stale: {text:?}");
        assert!(
            !text.contains("SENTINEL"),
            "uncached pass left the row cache patchable: {text:?}"
        );
        return;
    }
    panic!("never saw bravo");
}
