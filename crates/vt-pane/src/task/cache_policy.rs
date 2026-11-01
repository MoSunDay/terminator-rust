//! Frame-cache invalidation policy for [`Session`](super::Session):
//! what may rebuild lazily, what must rebuild fully, and when a
//! bounded row span (selection edits) is allowed to scope the rebuild.
//!
//! The engine's own dirty tracking is the patch basis for content, but
//! `set_selection` flips it to FULL wholesale - so selection scoping
//! runs on session-side bookkeeping (`content_epoch` freshness) and
//! deliberately ignores the engine layers while a fresh span is
//! pending. See `mouse::select_drag` for the producer side.

use anyhow::Result;

use super::Session;
use crate::term::{snapshot_frame_into, Frame};
/// The render snapshot, cached: rebuilt only after something visibly
/// changed (pty bytes via [`pump`], [`resize`], selection/viewport edits
/// or a theme sync invalidating via [`invalidate_frame`]). Rendering
/// paths use this; `frame` stays the uncached fresh snapshot.
///
/// PERF: when only PTY output invalidated the cache, the rebuild goes
/// through the engine's dirty-row tracking and patches just the rows that
/// changed (a 10k-cell pane then costs a few hundred FFI calls instead of
/// ~50k), reusing the cached grid's cells and strings in place.
pub fn frame_cached(sess: &mut Session) -> &Frame {
    if !sess.cache_valid {
        let full = sess.cache_full;
        // Only a span with no content arrival behind it may bypass the
        // engine's dirty state; a stale one would hide real changes.
        let pending = match (sess.pending_rows, sess.content_epoch) {
            (Some((span, epoch)), now) if epoch == now => Some(span),
            _ => None,
        };
        let built = snapshot_frame_into(
            &mut sess.term,
            &mut sess.render_state,
            &mut sess.row_it,
            &mut sess.cell_it,
            &mut sess.cache,
            full,
            pending,
        );
        bank_snapshot(sess, built);
    }
    &sess.cache
}

/// Bank a `snapshot_frame_into` outcome into the cache policy: success
/// publishes the frame, failure keeps the cache invalid for a FULL rebuild.
pub(crate) fn bank_snapshot(sess: &mut Session, built: Result<()>) {
    match built {
        Ok(()) => {
            sess.cache_valid = true;
            sess.cache_full = false;
            // The rebuild consumed the row span alongside the engine's
            // own dirty layers; keeping it would re-invalidate forever.
            sess.pending_rows = None;
            // Every row flag was cleared while passing and the global
            // layer was consumed by the row iterator update.
            sess.engine_clean = true;
            sess.sel_dirt = false;
        }
        Err(e) => {
            log::warn!("frame snapshot: {e}");
            // The frame may be half-updated: drop it AND keep the
            // cache invalid, so the next call rebuilds on the FULL
            // path (a "valid" cache would replay the half-built frame
            // forever - its generation never changes).
            sess.cache = Frame::default();
            sess.cache_valid = false;
            sess.cache_full = true;
        }
    }
}

/// Drop the cached snapshot and make the next rebuild re-read EVERY grid
/// row. Used whenever the change is invisible to the engine's per-row
/// dirty flags: selection edits, viewport scrolling, palette/theme swaps
/// and resizes.
pub fn invalidate_frame(sess: &mut Session) {
    sess.cache_valid = false;
    sess.cache_full = true;
    sess.frame_gen += 1;
    sess.content_epoch += 1;
    // Content-class change (or unknown): the engine may hold real dirt
    // and any pending selection span is stale.
    sess.engine_clean = false;
    sess.sel_dirt = false;
}

/// Invalidate a bounded span of viewport rows for the next cached
/// rebuild (selection edits: the caller bounded which rows can have
/// changed, see `mouse::select_drag`). Bumps `frame_gen` - the pane bg
/// layer merges selection runs, so it rebuilds whole - but row patching
/// stays inside the span plus whatever the engine itself dirtied. A
/// pending FULL rebuild stays dominant.
pub fn invalidate_frame_rows_span(sess: &mut Session, y0: u32, y1: u32) {
    sess.cache_valid = false;
    sess.frame_gen += 1;
    if sess.cache_full {
        return;
    }
    let span = (y0.min(y1), y0.max(y1));
    sess.pending_rows = Some(match sess.pending_rows {
        Some(((a, b), epoch)) => ((a.min(span.0), b.max(span.1)), epoch),
        None => (span, sess.content_epoch),
    });
}

/// Drop the cached snapshot after PTY output. The engine marks the rows
/// its content touched, so the rebuild patches only those (see
/// [`frame_cached`]); a pending full invalidation stays pending, because
/// row patching must never be trusted for a change it cannot see.
pub fn invalidate_frame_rows(sess: &mut Session) {
    sess.cache_valid = false;
    sess.frame_gen += 1;
    sess.content_epoch += 1;
    // Real content arrived: the engine holds genuine dirty rows.
    sess.engine_clean = false;
    sess.sel_dirt = false;
}

/// Current frame-cache generation: changes whenever something visibly
/// invalidated the snapshot, so render-side caches can key on it.
pub fn frame_gen(sess: &Session) -> u64 {
    sess.frame_gen
}
