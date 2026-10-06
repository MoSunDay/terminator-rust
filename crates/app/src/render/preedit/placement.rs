//! Input placement differs from the drawn cursor at a pending soft wrap.
use std::borrow::Cow;

use egui::{Pos2, Rect, Vec2};
use vt_pane::Frame;

use crate::state::CellSize;

/// `(column, row, preview scroll)`. Never changes terminal state.
pub fn position(frame: &Frame, first_span: u16) -> (u16, u16, bool) {
    let c = frame.cursor;
    let wraps = c.autowrap
        && (c.pending_wrap || u32::from(c.x) + u32::from(first_span) > u32::from(frame.cols));
    if wraps && frame.cols > 0 && frame.rows > 0 {
        let next = c.y.saturating_add(1);
        (0, next.min(frame.rows - 1), next >= frame.rows)
    } else {
        (c.x, c.y, false)
    }
}

pub fn cursor_rect(origin: Rect, frame: &Frame, cell: CellSize, span: u16) -> Option<Rect> {
    if !frame.cursor.visible || frame.cursor.x >= frame.cols || frame.cursor.y >= frame.rows {
        return None;
    }
    let (x, y, _) = position(frame, span);
    Some(Rect::from_min_size(
        Pos2::new(
            origin.left() + f32::from(x) * cell.w,
            origin.top() + f32::from(y) * cell.h,
        ),
        Vec2::new(cell.w, cell.h),
    ))
}

/// At the bottom, preview the row scroll that a commit will cause. Only the
/// painted copy scrolls: cancel immediately restores the untouched VT frame.
pub fn preview(frame: &Frame, composing: bool, span: u16) -> Cow<'_, Frame> {
    if !composing || !position(frame, span).2 {
        return Cow::Borrowed(frame);
    }
    let mut copy = frame.clone();
    if !copy.cells.is_empty() {
        copy.cells.remove(0);
        copy.cells
            .push(vec![Default::default(); usize::from(frame.cols)]);
    }
    Cow::Owned(copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_pane::{CellData, FrameCursor};

    fn frame(x: u16, y: u16, pending: bool) -> Frame {
        Frame {
            cols: 8,
            rows: 3,
            cursor: FrameCursor {
                x,
                y,
                visible: true,
                pending_wrap: pending,
                autowrap: true,
            },
            cells: vec![
                vec![
                    CellData {
                        text: "汉".into(),
                        ..Default::default()
                    };
                    8
                ];
                3
            ],
            ..Default::default()
        }
    }

    #[test]
    fn pending_and_wide_wraps_use_the_next_row() {
        assert_eq!(position(&frame(7, 0, true), 1), (0, 1, false));
        assert_eq!(position(&frame(7, 0, false), 1), (7, 0, false));
        assert_eq!(position(&frame(7, 0, false), 2), (0, 1, false));
        let mut no_wrap = frame(7, 0, false);
        no_wrap.cursor.autowrap = false;
        no_wrap.cursor.pending_wrap = true;
        assert_eq!(position(&no_wrap, 2), (7, 0, false));
    }

    #[test]
    fn bottom_preview_preserves_committed_text_and_cancellation_restores_the_frame() {
        let original = frame(7, 2, true);
        assert_eq!(position(&original, 2), (0, 2, true));
        let shown = preview(&original, true, 2);
        assert_eq!(shown.cells[1], original.cells[2]);
        assert!(shown.cells[2].iter().all(|c| c.text.is_empty()));
        assert_eq!(*preview(&original, false, 2), original);
        assert_eq!(original.cells[2][0].text, "汉");
    }
}
