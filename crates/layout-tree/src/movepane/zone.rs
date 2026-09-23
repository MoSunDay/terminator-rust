//! Drop-zone geometry: which side of a target pane a drag lands on,
//! and the screen region that zone occupies.

use crate::rect::{split_rect, Axis, Rect};
use crate::tree::{MAX_RATIO, MIN_RATIO};

/// Side of a target pane that a dragged pane would land on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropZone {
    /// Left edge: new vertical split, dragged pane first.
    Left,
    /// Right edge: new vertical split, dragged pane second.
    Right,
    /// Top edge: new horizontal split, dragged pane first.
    Top,
    /// Bottom edge: new horizontal split, dragged pane second.
    Bottom,
    /// Center square: swap the two pane ids in place.
    Center,
}

/// Which drop zone of `rect` the point `(x, y)` falls into: a centered
/// 50%-width square is the swap zone, the rest resolves to the nearest edge
/// (half of the pane along that axis). Points outside `rect` clamp to the
/// nearest zone.
pub fn zone_for(rect: &Rect, x: f32, y: f32) -> DropZone {
    let w = if rect.w > 0.0 { rect.w } else { 1.0 };
    let h = if rect.h > 0.0 { rect.h } else { 1.0 };
    let dx = ((x - rect.x) / w).clamp(0.0, 1.0);
    let dy = ((y - rect.y) / h).clamp(0.0, 1.0);
    let ex = (dx - 0.5).abs();
    let ey = (dy - 0.5).abs();
    if ex <= 0.25 && ey <= 0.25 {
        DropZone::Center
    } else if ex > ey {
        if dx < 0.5 {
            DropZone::Left
        } else {
            DropZone::Right
        }
    } else if dy < 0.5 {
        DropZone::Top
    } else {
        DropZone::Bottom
    }
}

/// The region the dragged pane would occupy when dropped in `zone`: the edge
/// share of `rect` at split `ratio` (clamped to `MIN_RATIO..=MAX_RATIO`),
/// the whole rect for [`DropZone::Center`].
pub fn zone_rect(rect: &Rect, zone: DropZone, ratio: f32) -> Rect {
    let ratio = ratio.clamp(MIN_RATIO, MAX_RATIO);
    match zone {
        DropZone::Center => *rect,
        DropZone::Left => split_rect(*rect, Axis::Vertical, ratio).0,
        DropZone::Right => split_rect(*rect, Axis::Vertical, ratio).1,
        DropZone::Top => split_rect(*rect, Axis::Horizontal, ratio).0,
        DropZone::Bottom => split_rect(*rect, Axis::Horizontal, ratio).1,
    }
}
