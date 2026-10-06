//! XIM supplies a point, but avoiding the composing row requires a rectangle.
//! Use the visible panel's measured size; no input events or text are changed.
use egui::{Pos2, Rect};

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
