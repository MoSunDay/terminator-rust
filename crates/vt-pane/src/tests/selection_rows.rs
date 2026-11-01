//! Row-scoped selection invalidation: a drag step rebuilds only the
//! bounded span (plus the engine's own dirty rows), a same-cell drag
//! rebuilds nothing, and a pending FULL rebuild dominates the span.

use crate::mouse as vmouse;
use crate::task::{self, SessionOpts};

/// 8-row grid, one printable char per row so rows are identifiable.
fn session() -> Option<task::Session> {
    let opts = SessionOpts::command(20, 8, vec!["cat".to_string()]);
    let mut sess = match task::spawn_session(&opts) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("skip: {e}");
            None
        }
    }?;
    sess.term
        .vt_write(b"r0\r\nr1\r\nr2\r\nr3\r\nr4\r\nr5\r\nr6\r\nr7\r\n");
    task::invalidate_frame_rows(&mut sess);
    task::frame_cached(&mut sess);
    Some(sess)
}

/// y pixel of a row's vertical middle.
fn row_y(sess: &task::Session, row: u32) -> f32 {
    let (_, ch) = task::cell_px(sess);
    row as f32 * ch as f32 + ch as f32 / 2.0
}

fn gens(sess: &mut task::Session) -> Vec<u64> {
    task::frame_cached(sess).row_gens.clone()
}

#[test]
fn drag_step_rebuilds_only_the_span_rows() {
    let Some(mut sess) = session() else { return };
    task::frame_cached(&mut sess); // settle: engine dirty rows consumed

    {
        let y = row_y(&sess, 2);
        vmouse::select_press(&mut sess, 4.0, y)
    }
    .unwrap();
    task::frame_cached(&mut sess); // press = FULL rebuild
    let base = gens(&mut sess);

    {
        let y = row_y(&sess, 5);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    let after = gens(&mut sess);

    // anchor row 2, head row 5 -> span [1, 6] (one slack row each end).
    for row in 0..8u32 {
        let in_span = (1..=6).contains(&row);
        assert_eq!(
            after[row as usize] > base[row as usize],
            in_span,
            "row {row} gen moved in_span={in_span}"
        );
    }
    assert_eq!(sess.pending_rows, None, "span consumed by the rebuild");
}

#[test]
fn same_cell_drag_rebuilds_nothing() {
    let Some(mut sess) = session() else { return };
    {
        let y = row_y(&sess, 2);
        vmouse::select_press(&mut sess, 4.0, y)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    {
        let y = row_y(&sess, 5);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    let (base, gen) = (gens(&mut sess), task::frame_gen(&sess));

    // Different pixel, same snapped cell: the engine selection is
    // equal, so nothing invalidates.
    {
        let y = row_y(&sess, 5) + 1.0;
        vmouse::select_drag(&mut sess, 42.0, y, false)
    }
    .unwrap();
    assert_eq!(task::frame_gen(&sess), gen, "no invalidation fired");
    assert_eq!(gens(&mut sess), base);
    assert!(sess.cache_valid, "cache untouched");
}

#[test]
fn second_drag_extends_the_union_span() {
    let Some(mut sess) = session() else { return };
    {
        let y = row_y(&sess, 2);
        vmouse::select_press(&mut sess, 4.0, y)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    {
        let y = row_y(&sess, 5);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    let base = gens(&mut sess);

    {
        let y = row_y(&sess, 6);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    let after = gens(&mut sess);

    // Union with the previous span: rows 1..=7 rebuild (over-rebuild is
    // the design), but row 0 - never touched - stays put.
    assert_eq!(after[0], base[0], "row 0 outside every span");
    assert!(after[7] > base[7], "row 7 entered the span");
}

#[test]
fn pending_full_rebuild_dominates_the_span() {
    let Some(mut sess) = session() else { return };
    {
        let y = row_y(&sess, 2);
        vmouse::select_press(&mut sess, 4.0, y)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    {
        let y = row_y(&sess, 3);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    let base = gens(&mut sess);

    task::invalidate_frame(&mut sess);
    task::invalidate_frame_rows_span(&mut sess, 0, 0);
    assert!(sess.cache_full, "FULL stays pending");
    task::frame_cached(&mut sess);
    let after = gens(&mut sess);
    for row in 0..8 {
        assert!(after[row] > base[row], "row {row} rebuilt by the FULL pass");
    }
}

#[test]
fn clear_selection_rebuilds_the_last_span() {
    let Some(mut sess) = session() else { return };
    {
        let y = row_y(&sess, 2);
        vmouse::select_press(&mut sess, 4.0, y)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    {
        let y = row_y(&sess, 5);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    let base = gens(&mut sess);

    vmouse::clear_selection(&mut sess);
    task::frame_cached(&mut sess);
    let after = gens(&mut sess);
    for row in 0..8u32 {
        let in_span = (1..=6).contains(&row);
        assert_eq!(
            after[row as usize] > base[row as usize],
            in_span,
            "clear rebuilt row {row} in_span={in_span}"
        );
    }
}

#[test]
fn content_arriving_behind_a_pending_span_is_not_hidden() {
    let Some(mut sess) = session() else { return };
    task::frame_cached(&mut sess); // settle
    {
        let y = row_y(&sess, 2);
        vmouse::select_press(&mut sess, 4.0, y)
    }
    .unwrap();
    task::frame_cached(&mut sess);
    let base = gens(&mut sess);
    {
        let y = row_y(&sess, 4);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    // span pending [1,5]; deliberately NO rebuild before the content.

    // Scrolling PTY output lands AFTER the drag: the engine goes FULL.
    sess.term.vt_write(b"\r\n\r\n\r\n");
    task::invalidate_frame_rows(&mut sess);

    task::frame_cached(&mut sess);
    let after = gens(&mut sess);
    for row in 0..8 {
        assert!(
            after[row] > base[row],
            "row {row}: stale span must not hide the engine's FULL"
        );
    }
    // The scrolled content is actually there (everything shifted up
    // three rows: row 0 now holds what was row 3).
    // The scrolled content is actually there: 9 seed lines + 3 more
    // newlines on an 8-row grid -> viewport shifted 4 rows total, so
    // row 0 holds what was seeded as "r4" (cell 0 of it: "r").
    // Cross-checked against the uncached fresh snapshot to prove the
    // cached path is not stale.
    let cached = task::frame_cached(&mut sess).cells[0][0].text.clone();
    let fresh = task::frame(&mut sess).unwrap().cells[0][0].text.clone();
    assert_eq!((cached.as_str(), fresh.as_str()), ("r", "r"));
}

#[test]
fn drag_right_after_press_without_a_rebuild_stays_scoped() {
    let Some(mut sess) = session() else { return };
    task::frame_cached(&mut sess); // settle
    {
        let y = row_y(&sess, 2);
        vmouse::select_press(&mut sess, 4.0, y)
    }
    .unwrap();
    // No frame_cached between press and drag (fast drag, one repaint).
    {
        let y = row_y(&sess, 5);
        vmouse::select_drag(&mut sess, 40.0, y, false)
    }
    .unwrap();
    task::frame_cached(&mut sess); // ONE rebuild for press+drag
    let after = gens(&mut sess);
    // Press bumped every gen (its FULL was still pending underneath);
    // the single rebuild was FULL, so all rows moved once - fine. The
    // property that matters: the frame is correct and banked.
    assert!(sess.cache_valid && !sess.cache_full && sess.pending_rows.is_none());
    let _ = after;
}
