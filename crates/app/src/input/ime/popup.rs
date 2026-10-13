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

/// How far (root px) a panel may sit from its target and still count as
/// placed: sub-pixel rounding and 1px caret jitter must not re-issue
/// XMoveWindow.
pub const MOVE_TOLERANCE_PX: f32 = 2.0;

/// Idempotence latch for panel placement. `measured` is where the panel
/// actually is, `commanded` the last position WE ordered for it (None
/// until the first move), `target` where it should be now. Returns the
/// position to command, or None when the panel is already placed - by an
/// earlier command whose target has not meaningfully moved, or by a
/// coinciding native placement. Re-issuing XMoveWindow on an
/// already-placed panel is what fed the 60Hz tug-of-war with the IME
/// server (it repositions the panel on its own triggers, we pulled it
/// back every frame).
pub fn decide_move(measured: Pos2, commanded: Option<Pos2>, target: Pos2) -> Option<Pos2> {
    let near = |a: Pos2, b: Pos2| a.distance(b) <= MOVE_TOLERANCE_PX;
    // Our command landed and the target stayed put.
    if commanded.is_some_and(|c| near(c, target) && near(measured, c)) {
        return None;
    }
    // The panel's own placement already coincides with the target.
    if near(measured, target) {
        return None;
    }
    Some(target)
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

    #[test]
    fn decide_move_is_none_when_native_placement_coincides() {
        let target = pos2(300.0, 400.0);
        assert_eq!(decide_move(target, None, target), None);
    }

    #[test]
    fn decide_move_reissues_when_the_server_redocks_the_panel() {
        let target = pos2(300.0, 400.0);
        let redocked = pos2(4.0, 700.0); // fcitx5 re-docked at window bottom-left
        assert_eq!(decide_move(redocked, Some(target), target), Some(target));
    }

    #[test]
    fn decide_move_follows_a_moved_target() {
        let old = pos2(300.0, 400.0);
        let moved = pos2(310.0, 400.0);
        assert_eq!(decide_move(old, Some(old), moved), Some(moved));
    }

    #[test]
    fn decide_move_tolerates_single_pixel_jitter() {
        let commanded = pos2(300.0, 400.0);
        for delta in [-1.0_f32, 1.0] {
            let measured = commanded + vec2(delta, -delta);
            assert_eq!(decide_move(measured, Some(commanded), commanded), None);
        }
    }
}
