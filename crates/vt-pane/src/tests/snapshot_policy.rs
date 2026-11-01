//! `bank_snapshot` cache policy: a failed rebuild must never publish a
//! half-updated frame.

use crate::task::{self, SessionOpts};
use crate::term::Frame;

fn session() -> Option<task::Session> {
    let opts = SessionOpts::command(20, 4, vec!["cat".to_string()]);
    match task::spawn_session(&opts) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("skip: {e}");
            None
        }
    }
}

#[test]
fn failed_rebuild_keeps_cache_invalid_and_empty() {
    let Some(mut sess) = session() else { return };
    sess.term.vt_write(b"hello");
    let first = task::frame_cached(&mut sess);
    assert_eq!(first.cells[0][0].text, "h");
    assert!(sess.cache_valid && !sess.cache_full, "good build banked");

    task::bank_snapshot(&mut sess, Err(anyhow::anyhow!("boom")));

    assert!(!sess.cache_valid, "failed build stays invalid");
    assert!(sess.cache_full, "next rebuild takes the FULL path");
    assert_eq!(sess.cache, Frame::default(), "half-built frame dropped");

    // The next call rebuilds from the live terminal, not from the corpse.
    let again = task::frame_cached(&mut sess);
    assert_eq!(again.cells[0][0].text, "h");
    assert!(sess.cache_valid && !sess.cache_full, "retry banked");
}

#[test]
fn cached_frame_is_returned_without_a_rebuild() {
    let Some(mut sess) = session() else { return };
    sess.term.vt_write(b"hello");
    assert_eq!(task::frame_cached(&mut sess).cells[0][0].text, "h");
    // Content written behind the cache's back must NOT appear: a valid
    // cache is never rebuilt until something invalidates it.
    sess.term.vt_write(b"\r\nworld");
    let cached = task::frame_cached(&mut sess);
    assert!(cached.cells[1][0].text.is_empty());
    task::invalidate_frame(&mut sess);
    assert_eq!(task::frame_cached(&mut sess).cells[1][0].text, "w");
}

#[test]
fn row_gens_bump_only_the_rows_the_engine_reports() {
    let Some(mut sess) = session() else { return };
    sess.term.vt_write(b"one");
    let first = task::frame_cached(&mut sess);
    assert_eq!(first.row_gens.len(), usize::from(first.rows));
    assert!(
        first.row_gens.iter().all(|g| *g >= 1),
        "a fresh build leaves no zero gens (0 only exists in Frame::default)"
    );
    let before: Vec<u64> = first.row_gens.clone();
    // Same-row output: only the row the engine dirtied moves on.
    task::invalidate_frame_rows(&mut sess);
    sess.term.vt_write(b"X");
    let second = task::frame_cached(&mut sess);
    let moved: Vec<usize> = second
        .row_gens
        .iter()
        .zip(&before)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(y, _)| y)
        .collect();
    assert!(!moved.is_empty(), "the written row must bump");
    assert!(
        moved
            .iter()
            .all(|y| second.cells[*y].iter().any(|c| !c.text.is_empty())),
        "only rows with content may move (scroll would move all - fine)"
    );
    // A full invalidation bumps every row.
    let before_full: Vec<u64> = second.row_gens.clone();
    task::invalidate_frame(&mut sess);
    let third = task::frame_cached(&mut sess);
    assert!(
        third.row_gens.iter().zip(&before_full).all(|(a, b)| a != b),
        "full invalidation bumps every row gen"
    );
}
