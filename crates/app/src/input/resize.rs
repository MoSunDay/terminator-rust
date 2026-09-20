//! Borderless-window edge resizing: invisible pointer strips at the
//! viewport border drive the WM resize gesture (EWMH _NET_WM_MOVERESIZE).

use egui::{pos2, Id, Pos2, Rect, Sense, Ui};

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

/// Register the 8 border strips as the LAST widgets of a window pass:
/// being registered last they sit on top of everything, so a press on a
/// strip is stolen from chrome/panes automatically (widget hit-test).
pub fn strips(ui: &mut Ui, win: Rect) {
    use egui::ResizeDirection as R;
    // Plain edges first, inset by CORNER on the long axis so a corner
    // press never races a straight edge; the corner squares register
    // after (later = on top) and win the diagonal.
    let edges = [
        (
            Rect::from_min_max(
                pos2(win.left() + CORNER, win.top()),
                pos2(win.right() - CORNER, win.top() + EDGE),
            ),
            R::North,
        ),
        (
            Rect::from_min_max(
                pos2(win.left() + CORNER, win.bottom() - EDGE),
                pos2(win.right() - CORNER, win.bottom()),
            ),
            R::South,
        ),
        (
            Rect::from_min_max(
                pos2(win.left(), win.top() + CORNER),
                pos2(win.left() + EDGE, win.bottom() - CORNER),
            ),
            R::West,
        ),
        (
            Rect::from_min_max(
                pos2(win.right() - EDGE, win.top() + CORNER),
                pos2(win.right(), win.bottom() - CORNER),
            ),
            R::East,
        ),
    ];
    let corners = [
        (
            Rect::from_min_max(
                pos2(win.left(), win.top()),
                pos2(win.left() + CORNER, win.top() + CORNER),
            ),
            R::NorthWest,
        ),
        (
            Rect::from_min_max(
                pos2(win.right() - CORNER, win.top()),
                pos2(win.right(), win.top() + CORNER),
            ),
            R::NorthEast,
        ),
        (
            Rect::from_min_max(
                pos2(win.left(), win.bottom() - CORNER),
                pos2(win.left() + CORNER, win.bottom()),
            ),
            R::SouthWest,
        ),
        (
            Rect::from_min_max(
                pos2(win.right() - CORNER, win.bottom() - CORNER),
                pos2(win.right(), win.bottom()),
            ),
            R::SouthEast,
        ),
    ];
    for (k, (rect, dir)) in edges.into_iter().chain(corners).enumerate() {
        let resp = ui.interact(rect, Id::new("edge_resize").with(k), Sense::drag());
        if resp.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
        resp.on_hover_cursor(cursor_for(dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::ResizeDirection as R;

    fn win() -> Rect {
        Rect::from_min_size(pos2(100.0, 100.0), egui::vec2(400.0, 300.0))
    }

    #[test]
    fn dir_at_eight_directions() {
        let w = win();
        // Plain edges.
        assert_eq!(dir_at(w, pos2(300.0, 101.0)), Some(R::North));
        assert_eq!(dir_at(w, pos2(300.0, 399.0)), Some(R::South));
        assert_eq!(dir_at(w, pos2(101.0, 250.0)), Some(R::West));
        assert_eq!(dir_at(w, pos2(499.0, 250.0)), Some(R::East));
        // Corners beat edges.
        assert_eq!(dir_at(w, pos2(101.0, 101.0)), Some(R::NorthWest));
        assert_eq!(dir_at(w, pos2(499.0, 101.0)), Some(R::NorthEast));
        assert_eq!(dir_at(w, pos2(101.0, 399.0)), Some(R::SouthWest));
        assert_eq!(dir_at(w, pos2(499.0, 399.0)), Some(R::SouthEast));
    }

    #[test]
    fn dir_at_center_and_outside_none() {
        let w = win();
        assert_eq!(dir_at(w, w.center()), None);
        // Inside the corner square's x range but past its y range and off
        // every plain edge: dead center of the border ring.
        assert_eq!(dir_at(w, pos2(112.0, 130.0)), None);
        // Outside the window: never a resize zone.
        assert_eq!(dir_at(w, pos2(50.0, 250.0)), None);
        assert_eq!(dir_at(w, pos2(300.0, 95.0)), None);
        assert_eq!(dir_at(w, pos2(510.0, 410.0)), None);
    }

    #[test]
    fn cursor_for_maps_directions() {
        use egui::CursorIcon;
        assert_eq!(cursor_for(R::North), CursorIcon::ResizeVertical);
        assert_eq!(cursor_for(R::South), CursorIcon::ResizeVertical);
        assert_eq!(cursor_for(R::East), CursorIcon::ResizeHorizontal);
        assert_eq!(cursor_for(R::West), CursorIcon::ResizeHorizontal);
        assert_eq!(cursor_for(R::NorthEast), CursorIcon::ResizeNeSw);
        assert_eq!(cursor_for(R::SouthWest), CursorIcon::ResizeNeSw);
        assert_eq!(cursor_for(R::NorthWest), CursorIcon::ResizeNwSe);
        assert_eq!(cursor_for(R::SouthEast), CursorIcon::ResizeNwSe);
    }
}
