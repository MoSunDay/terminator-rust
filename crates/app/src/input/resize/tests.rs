use super::gesture::{moves_origin, resized_rect, Gesture, MIN_SIZE};
use super::hit::{cursor_for, dir_at};
use super::strips::escaped;
use egui::ResizeDirection as R;
use egui::{pos2, Pos2, Rect};

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

fn gesture(dir: R, pointer: Pos2) -> Gesture {
    Gesture {
        dir,
        pointer,
        rect: win(),
    }
}

#[test]
fn escape_guard_lets_following_and_boundary_pointer_live() {
    let w = win();
    // exactly on the west border: still tracked
    assert!(!escaped(w, Some(pos2(w.left(), w.top() + 10.0))));
    // just outside within the margin (screen-edge chase): still tracked
    assert!(!escaped(w, Some(pos2(w.right() + 90.0, w.top() + 10.0))));
    // PointerGone
    assert!(escaped(w, None));
}

#[test]
fn escape_guard_kills_a_pointer_far_outside() {
    let w = win();
    assert!(escaped(w, Some(pos2(w.left() - 200.0, w.top() + 10.0))));
    assert!(escaped(w, Some(pos2(w.right() + 150.0, w.top() + 10.0))));
}

#[test]
fn resized_rect_east_west_follow_pointer() {
    // East edge +100: only the right edge moves.
    let g = gesture(R::East, pos2(500.0, 250.0));
    let r = resized_rect(&g, pos2(600.0, 250.0));
    assert_eq!(
        r,
        Rect::from_min_size(pos2(100.0, 100.0), egui::vec2(500.0, 300.0))
    );
    // West edge -50: the LEFT edge moves (origin follows), width grows.
    let g = gesture(R::West, pos2(100.0, 250.0));
    let r = resized_rect(&g, pos2(50.0, 250.0));
    assert_eq!(
        r,
        Rect::from_min_size(pos2(50.0, 100.0), egui::vec2(450.0, 300.0))
    );
}

#[test]
fn resized_rect_corners_move_both_axes() {
    let g = gesture(R::SouthEast, pos2(500.0, 400.0));
    let r = resized_rect(&g, pos2(560.0, 470.0));
    assert_eq!(
        r,
        Rect::from_min_size(pos2(100.0, 100.0), egui::vec2(460.0, 370.0))
    );
    // NorthWest drag: origin chases the pointer, far corner is pinned.
    let g = gesture(R::NorthWest, pos2(100.0, 100.0));
    let r = resized_rect(&g, pos2(60.0, 60.0));
    assert_eq!(
        r,
        Rect::from_min_size(pos2(60.0, 60.0), egui::vec2(440.0, 340.0))
    );
}

#[test]
fn resized_rect_clamps_to_min_on_grabbed_side() {
    // Dragging the east edge far LEFT past the west edge: the west
    // edge stays, width floors at MIN_SIZE.
    let g = gesture(R::East, pos2(500.0, 250.0));
    let r = resized_rect(&g, pos2(-1000.0, 250.0));
    assert_eq!(r.min, pos2(100.0, 100.0));
    assert_eq!(r.size(), MIN_SIZE);
    // Dragging the west edge far RIGHT: the east edge stays.
    let g = gesture(R::West, pos2(100.0, 250.0));
    let r = resized_rect(&g, pos2(5000.0, 250.0));
    assert_eq!(r.max, pos2(500.0, 400.0));
    assert_eq!(r.size(), MIN_SIZE);
    // North dragged below the south edge: south stays.
    let g = gesture(R::North, pos2(300.0, 100.0));
    let r = resized_rect(&g, pos2(300.0, 5000.0));
    assert_eq!(r.max, pos2(500.0, 400.0));
    assert_eq!(r.size(), MIN_SIZE);
}

#[test]
fn moves_origin_flags_north_and_west() {
    assert!(moves_origin(R::North));
    assert!(moves_origin(R::West));
    assert!(moves_origin(R::NorthWest));
    assert!(moves_origin(R::NorthEast));
    assert!(!moves_origin(R::South));
    assert!(!moves_origin(R::East));
    assert!(!moves_origin(R::SouthEast));
    assert!(!moves_origin(R::SouthWest));
}
