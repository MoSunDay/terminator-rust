//! Local viewport (scrollback review) actions and geometry.
//!
//! Thin pure helpers over [`Session`]: the callers (keyboard review keys,
//! the scrollbar indicator) stay free of libghostty-vt types.

use libghostty_vt::terminal::ScrollViewport;

use crate::Session;

/// Signed viewport delta for one page-step of `rows` visible rows (one
/// overlapping context line between consecutive pages).
///
/// A page is `rows - 1` lines so consecutive pages keep one line of
/// context; tiny viewports (rows < 2) still step a single line so the
/// key never becomes a no-op.
pub fn page_delta(rows: u16, up: bool) -> isize {
    let page = (u32::from(rows.max(2)) as isize - 1).max(1);
    if up {
        -page
    } else {
        page
    }
}

/// Scroll the viewport by signed `lines` (up = negative); the engine clamps.
pub fn scroll_by(sess: &mut Session, lines: isize) {
    sess.term.scroll_viewport(ScrollViewport::Delta(lines));
}

/// Jump to the top of the scrollback history.
pub fn scroll_top(sess: &mut Session) {
    sess.term.scroll_viewport(ScrollViewport::Top);
}

/// Return to the live area (what follow_output does after typing).
pub fn scroll_bottom(sess: &mut Session) {
    sess.term.scroll_viewport(ScrollViewport::Bottom);
}

/// Viewport pinned to the live area (false = user scrolled into history).
pub fn pinned(sess: &Session) -> bool {
    sess.term.viewport_active().unwrap_or(true)
}

/// Scrollbar geometry `(offset, total, len)` in rows, for indicators.
pub fn geometry(sess: &Session) -> Option<(u64, u64, u64)> {
    sess.term
        .scrollbar()
        .ok()
        .map(|s| (s.offset, s.total, s.len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_steps_overlap_by_one_line() {
        assert_eq!(page_delta(24, true), -23);
        assert_eq!(page_delta(24, false), 23);
        assert_eq!(page_delta(2, true), -1);
        assert_eq!(page_delta(2, false), 1);
    }

    #[test]
    fn tiny_viewports_still_step_one_line() {
        // rows < 2 would otherwise compute a 0-line page (a stuck key).
        assert_eq!(page_delta(1, true), -1);
        assert_eq!(page_delta(1, false), 1);
        assert_eq!(page_delta(0, false), 1);
    }
}
