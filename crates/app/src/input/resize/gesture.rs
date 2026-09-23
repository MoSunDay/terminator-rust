//! In-flight resize gesture: the press-time anchor plus the pure rect
//! math that follows the pointer while pinning the opposite edges.

use egui::{Pos2, Rect, Vec2};

/// Smallest window an edge gesture may produce (points).
pub const MIN_SIZE: Vec2 = Vec2::new(400.0, 300.0);

/// Anchor of an in-flight edge resize: screen-space (points) pointer
/// and outer rect captured at the press. Every frame recomputes from
/// this anchor, so ConfigureNotify lag never accumulates into drift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gesture {
    pub dir: egui::ResizeDirection,
    pub pointer: Pos2,
    pub rect: Rect,
}

/// True when the direction moves the window origin (needs
/// `OuterPosition` in addition to `InnerSize`).
pub fn moves_origin(dir: egui::ResizeDirection) -> bool {
    use egui::ResizeDirection as R;
    matches!(dir, R::North | R::West | R::NorthEast | R::NorthWest)
}

/// New outer rect for the gesture with the pointer now at screen
/// position `now`: the grabbed edges follow the pointer, the opposite
/// edges stay put, and the result never shrinks below `MIN_SIZE`
/// (the clamp pins the OPPOSITE edge and pushes the grabbed one back).
pub fn resized_rect(g: &Gesture, now: Pos2) -> Rect {
    use egui::ResizeDirection as R;
    let d = now.to_vec2() - g.pointer.to_vec2();
    let mut min = g.rect.min;
    let mut max = g.rect.max;
    match g.dir {
        R::East => max.x += d.x,
        R::West => min.x += d.x,
        R::South => max.y += d.y,
        R::North => min.y += d.y,
        R::NorthEast => {
            max.x += d.x;
            min.y += d.y;
        }
        R::NorthWest => {
            min.x += d.x;
            min.y += d.y;
        }
        R::SouthEast => {
            max.x += d.x;
            max.y += d.y;
        }
        R::SouthWest => {
            min.x += d.x;
            max.y += d.y;
        }
    }
    let west = matches!(g.dir, R::West | R::NorthWest | R::SouthWest);
    if max.x - min.x < MIN_SIZE.x {
        if west {
            min.x = max.x - MIN_SIZE.x;
        } else {
            max.x = min.x + MIN_SIZE.x;
        }
    }
    let north = matches!(g.dir, R::North | R::NorthEast | R::NorthWest);
    if max.y - min.y < MIN_SIZE.y {
        if north {
            min.y = max.y - MIN_SIZE.y;
        } else {
            max.y = min.y + MIN_SIZE.y;
        }
    }
    Rect::from_min_max(min, max)
}
