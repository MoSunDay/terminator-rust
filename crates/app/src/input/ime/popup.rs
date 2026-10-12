//! XIM supplies a point, but avoiding the composing row requires a rectangle.
//! Use the visible panel's measured size; no input events or text are changed.
use egui::{Pos2, Rect};

/// How far (px) a panel may sit from the caret row and still count as
/// following it - anything farther is treated as a parked panel.
pub const NEAR_ROW_PX: f32 = 24.0;

/// Parked-panel rescue: some XIM servers ignore `XNSpotLocation` while
/// preedit callbacks are registered (libX11 bug 1580 - the very style
/// winit negotiates), leaving the candidate window docked at the
/// window's bottom-left. While composing, place such a panel just
/// below the caret row (above it when the monitor has no room), with a
/// 1px seam so it never covers the row itself. Panels already within
/// `NEAR_ROW_PX` of the row keep their native placement.
pub fn follow_row(caret: Rect, panel: Rect, monitor: Rect) -> Option<Pos2> {
    let near =
        panel.top() <= caret.bottom() + NEAR_ROW_PX && panel.bottom() + NEAR_ROW_PX >= caret.top();
    if near {
        return None;
    }
    let y = if caret.bottom() + 1.0 + panel.height() <= monitor.bottom() {
        caret.bottom() + 1.0
    } else if caret.top() - panel.height() >= monitor.top() {
        caret.top() - panel.height()
    } else {
        return None; // No vertical placement fits; don't move it off-screen.
    };
    let x = caret.left().clamp(
        monitor.left(),
        (monitor.right() - panel.width()).max(monitor.left()),
    );
    Some(Pos2::new(x, y))
}

/// Preserve an already safe placement. Move an overlapping panel to available
/// space on the caret's monitor, including monitors with negative coordinates.
pub fn avoid_row(caret: Rect, panel: Rect, monitor: Rect) -> Option<Pos2> {
    if panel.bottom() <= caret.top() || panel.top() >= caret.bottom() {
        return None;
    }
    let y = if caret.bottom() + panel.height() <= monitor.bottom() {
        caret.bottom()
    } else if caret.top() - panel.height() >= monitor.top() {
        caret.top() - panel.height()
    } else {
        return None; // No vertical placement fits; don't move it off-screen.
    };
    let x = caret.left().clamp(
        monitor.left(),
        (monitor.right() - panel.width()).max(monitor.left()),
    );
    Some(Pos2::new(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn parked_panel_follows_the_row_with_a_seam() {
        let monitor = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        let caret = Rect::from_min_size(pos2(279.0, 106.0), vec2(9.0, 20.0));
        // libX11-parked panel: window bottom-left, far from the row.
        let parked = Rect::from_min_size(pos2(0.0, 736.0), vec2(314.0, 64.0));
        let at = follow_row(caret, parked, monitor).expect("rescue parked panel");
        assert_eq!(at, pos2(279.0, 127.0)); // caret.bottom() + 1px seam
                                            // Once adjacent the panel is near the row: keep native placement.
        assert!(follow_row(caret, Rect::from_min_size(at, parked.size()), monitor).is_none());
        // Screen-bottom caret flips to above the row (a panel docked at
        // the bottom is already near that row, so park this one at the
        // top before rescuing it).
        let low = Rect::from_min_size(pos2(100.0, 770.0), vec2(9.0, 20.0));
        let top_parked = Rect::from_min_size(pos2(0.0, 10.0), vec2(314.0, 64.0));
        let at = follow_row(low, top_parked, monitor).expect("flip above");
        assert_eq!(at, pos2(100.0, 770.0 - 64.0));
        // No room either way (panel taller than the monitor): stay put.
        let huge = Rect::from_min_size(pos2(0.0, 736.0), vec2(314.0, 900.0));
        assert!(follow_row(caret, huge, monitor).is_none());
    }

    #[test]
    fn actual_panel_size_controls_above_below_and_monitor_clamping() {
        let monitor = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0));
        for height in [64.0, 220.0] {
            let caret = Rect::from_min_size(pos2(1152.0, 768.0), vec2(9.0, 18.0));
            let panel = Rect::from_min_size(pos2(912.0, 786.0 - height), vec2(288.0, height));
            let at = avoid_row(caret, panel, monitor).expect("move overlapping panel");
            assert_eq!(at, pos2(912.0, 768.0 - height));
            assert!(avoid_row(caret, Rect::from_min_size(at, panel.size()), monitor).is_none());
        }
        let offset = vec2(-1200.0, -100.0);
        let caret = Rect::from_min_size(pos2(30.0, 100.0), vec2(9.0, 18.0));
        let panel = Rect::from_min_size(caret.min, vec2(288.0, 64.0));
        assert_eq!(avoid_row(caret, panel, monitor), Some(caret.left_bottom()));
        assert_eq!(
            avoid_row(
                caret.translate(offset),
                panel.translate(offset),
                monitor.translate(offset)
            ),
            Some(caret.left_bottom() + offset)
        );
    }
}
