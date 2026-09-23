//! Border hit-testing: which resize direction a border pointer is in,
//! and the cursor shape that direction wants.

use egui::{Pos2, Rect};

/// Strip thickness (points) along a plain window edge.
pub const EDGE: f32 = 6.0;
/// Corner square side: diagonal grips beat plain edges.
pub const CORNER: f32 = 14.0;

/// Which resize direction `pos` is in at the border of `win`, if any.
/// Corners (`CORNER` x `CORNER` squares) win over plain edges (`EDGE`
/// strips); points outside the window or in the middle report `None`.
pub fn dir_at(win: Rect, pos: Pos2) -> Option<egui::ResizeDirection> {
    // Distances to each side; negative = outside the window entirely.
    let l = pos.x - win.left();
    let r = win.right() - pos.x;
    let t = pos.y - win.top();
    let b = win.bottom() - pos.y;
    if l < 0.0 || r < 0.0 || t < 0.0 || b < 0.0 {
        return None;
    }
    let edge = |d: f32| d <= EDGE;
    let corner = |d: f32| d <= CORNER;
    Some(match () {
        _ if corner(l) && corner(t) => egui::ResizeDirection::NorthWest,
        _ if corner(r) && corner(t) => egui::ResizeDirection::NorthEast,
        _ if corner(l) && corner(b) => egui::ResizeDirection::SouthWest,
        _ if corner(r) && corner(b) => egui::ResizeDirection::SouthEast,
        _ if edge(t) => egui::ResizeDirection::North,
        _ if edge(b) => egui::ResizeDirection::South,
        _ if edge(l) => egui::ResizeDirection::West,
        _ if edge(r) => egui::ResizeDirection::East,
        _ => return None,
    })
}

/// Cursor shape for a resize direction. egui only carries the four
/// two-way arrow cursors, so opposite directions share one.
pub fn cursor_for(dir: egui::ResizeDirection) -> egui::CursorIcon {
    use egui::CursorIcon;
    use egui::ResizeDirection as R;
    match dir {
        R::North | R::South => CursorIcon::ResizeVertical,
        R::East | R::West => CursorIcon::ResizeHorizontal,
        R::NorthEast | R::SouthWest => CursorIcon::ResizeNeSw,
        R::NorthWest | R::SouthEast => CursorIcon::ResizeNwSe,
    }
}
