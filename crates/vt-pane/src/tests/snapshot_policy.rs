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
