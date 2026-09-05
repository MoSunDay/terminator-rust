//! Pure 2-D geometry for pane layouts.
//!
//! Axis semantics (terminator convention):
//!
//! - [`Axis::Horizontal`]: a split divides a rect into **top / bottom**
//!   halves. The divider is a horizontal line and the children are stacked
//!   vertically, so the split runs along the rect's height.
//! - [`Axis::Vertical`]: a split divides a rect into **left / right**
//!   halves. The divider is a vertical line and the children sit side by
//!   side, so the split runs along the rect's width.

/// Orientation of a split divider (see the module docs for the convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Horizontal divider; children stack top / bottom.
    Horizontal,
    /// Vertical divider; children sit left / right.
    Vertical,
}

/// Axis-aligned rectangle with its origin at the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

/// Area of `r` (`w * h`, not clamped).
pub fn rect_area(r: Rect) -> f32 {
    r.w * r.h
}

/// True when the closed rect `r` contains the point `(px, py)` (edges count).
pub fn rect_contains(r: Rect, px: f32, py: f32) -> bool {
    px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
}

/// Center point of `r`.
pub fn rect_center(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

/// Splits `r` along `axis` into `(first, second)`; `first` (top/left) gets
/// `ratio` of the total length. No divider gap is reserved.
/// `ratio` outside `0.0..=1.0` is clamped.
pub fn split_rect(r: Rect, axis: Axis, ratio: f32) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.0, 1.0);
    match axis {
        Axis::Horizontal => {
            let first_h = r.h * ratio;
            (
                Rect { h: first_h, ..r },
                Rect { y: r.y + first_h, h: r.h - first_h, ..r },
            )
        }
        Axis::Vertical => {
            let first_w = r.w * ratio;
            (
                Rect { w: first_w, ..r },
                Rect { x: r.x + first_w, w: r.w - first_w, ..r },
            )
        }
    }
}

/// Like [`split_rect`], but reserves a `gap`-thick divider between the
/// halves: `first` gets `ratio * (len - gap)`, the divider occupies the
/// sliver in between and `second` gets the remainder. Lengths never drop
/// below zero.
pub fn split_rect_gapped(r: Rect, axis: Axis, ratio: f32, gap: f32) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.0, 1.0);
    let gap = gap.max(0.0);
    match axis {
        Axis::Horizontal => {
            let usable = (r.h - gap).max(0.0);
            let first_h = usable * ratio;
            (
                Rect { h: first_h, ..r },
                Rect {
                    y: r.y + first_h + gap,
                    h: (r.h - first_h - gap).max(0.0),
                    ..r
                },
            )
        }
        Axis::Vertical => {
            let usable = (r.w - gap).max(0.0);
            let first_w = usable * ratio;
            (
                Rect { w: first_w, ..r },
                Rect {
                    x: r.x + first_w + gap,
                    w: (r.w - first_w - gap).max(0.0),
                    ..r
                },
            )
        }
    }
}

/// Insets every side of `r` by `amount` (clamped at zero width/height).
pub fn shrink(r: Rect, amount: f32) -> Rect {
    let amount = amount.max(0.0);
    Rect {
        x: r.x + amount,
        y: r.y + amount,
        w: (r.w - 2.0 * amount).max(0.0),
        h: (r.h - 2.0 * amount).max(0.0),
    }
}

/// True when the point `(px, py)` lies within `tol` of a divider line of
/// orientation `axis` positioned at `pos`: for [`Axis::Horizontal`] `pos`
/// is a `y` coordinate (horizontal line), for [`Axis::Vertical`] it is an
/// `x` coordinate (vertical line).
pub fn near_divider(px: f32, py: f32, axis: Axis, pos: f32, tol: f32) -> bool {
    match axis {
        Axis::Horizontal => (py - pos).abs() <= tol,
        Axis::Vertical => (px - pos).abs() <= tol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn contains_is_closed_on_far_edges() {
        let r = Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 };
        assert!(rect_contains(r, 1.0, 2.0));
        assert!(rect_contains(r, 4.0, 6.0));
        assert!(!rect_contains(r, 4.01, 6.0));
        assert!(!rect_contains(r, 0.0, 0.0));
    }

    #[test]
    fn center_and_area() {
        let r = Rect { x: 0.0, y: 0.0, w: 10.0, h: 6.0 };
        assert_eq!(rect_center(r), (5.0, 3.0));
        assert!(close(rect_area(r), 60.0));
    }

    #[test]
    fn split_horizontal_is_top_bottom() {
        let r = Rect { x: 0.0, y: 0.0, w: 100.0, h: 90.0 };
        let (top, bottom) = split_rect(r, Axis::Horizontal, 0.3);
        assert!(close(top.h, 27.0));
        assert!(close(top.w, 100.0));
        assert!(close(bottom.y, 27.0));
        assert!(close(bottom.h, 63.0));
    }

    #[test]
    fn split_vertical_is_left_right() {
        let r = Rect { x: 5.0, y: 0.0, w: 90.0, h: 100.0 };
        let (left, right) = split_rect(r, Axis::Vertical, 0.5);
        assert!(close(left.w, 45.0));
        assert!(close(left.x, 5.0));
        assert!(close(right.x, 50.0));
        assert!(close(right.w, 45.0));
    }

    #[test]
    fn gapped_split_reserves_divider_room() {
        let r = Rect { x: 0.0, y: 0.0, w: 100.0, h: 90.0 };
        let (a, b) = split_rect_gapped(r, Axis::Horizontal, 0.5, 10.0);
        assert!(close(a.h, 40.0));
        assert!(close(b.y, 50.0));
        assert!(close(b.h, 40.0));
        // Vertical: first = 0.5 * (100 - 10) = 45, gap, second starts at 55.
        let (a, b) = split_rect_gapped(r, Axis::Vertical, 0.5, 10.0);
        assert!(close(a.w, 45.0));
        assert!(close(b.x, 55.0));
        assert!(close(b.w, 45.0));
    }

    #[test]
    fn shrink_insets_all_sides() {
        let r = shrink(Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 }, 2.0);
        assert_eq!(r, Rect { x: 2.0, y: 2.0, w: 6.0, h: 6.0 });
        let flat = shrink(Rect { x: 0.0, y: 0.0, w: 2.0, h: 5.0 }, 4.0);
        assert!(flat.w <= 0.0);
        assert!(flat.h <= 0.0);
    }

    #[test]
    fn near_divider_checks_line_orientation() {
        assert!(near_divider(3.0, 10.0, Axis::Horizontal, 10.5, 1.0));
        assert!(!near_divider(3.0, 12.0, Axis::Horizontal, 10.5, 1.0));
        assert!(near_divider(7.0, 42.0, Axis::Vertical, 6.5, 1.0));
        assert!(!near_divider(5.0, 42.0, Axis::Vertical, 6.5, 1.0));
    }
}
