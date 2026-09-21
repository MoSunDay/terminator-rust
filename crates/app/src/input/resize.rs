//! Borderless-window edge resizing: invisible pointer strips at the
//! viewport border drive an APP-OWNED resize. The previous design sent
//! `ViewportCommand::BeginResize` (EWMH _NET_WM_MOVERESIZE), handing
//! gesture termination to the WM: the WM's pointer grab also swallows
//! the ButtonRelease, so egui keeps believing the button is held (the
//! next gesture then cannot latch) and a WM that misses the release
//! keeps resizing after mouseup. Applying `InnerSize`/`OuterPosition`
//! each dragged frame keeps the release - and with it termination - on
//! our side of the fence: the window stops following the pointer the
//! moment the button goes up.

use egui::{pos2, Id, Pos2, Rect, Sense, Ui, Vec2};

/// Strip thickness (points) along a plain window edge.
pub const EDGE: f32 = 6.0;
/// Corner square side: diagonal grips beat plain edges.
pub const CORNER: f32 = 14.0;
/// Smallest window an edge gesture may produce (points).
pub const MIN_SIZE: Vec2 = Vec2::new(400.0, 300.0);

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

/// Screen-space pointer (points): viewport-local pointer plus the
/// viewport origin. Anchor and live position share this space, so a
/// window resizing under a held pointer still yields true deltas.
fn screen_pointer(ui: &Ui) -> Option<Pos2> {
    ui.input(|i| {
        let local = i.pointer.latest_pos()?;
        let origin = i.viewport().outer_rect.or(i.viewport().inner_rect)?;
        Some(origin.min + local.to_vec2())
    })
}

/// Register the 8 border strips as the LAST widgets of a window pass:
/// being registered last they sit on top of everything, so a press on a
/// strip is stolen from chrome/panes automatically (widget hit-test).
pub fn strips(ui: &mut Ui, win_id: u64, win: Rect, gesture: &mut Option<Gesture>) {
    // Polled once per frame while a gesture is live: on X11 this is
    // its ground truth; where unavailable the event path stands in.
    let polled = gesture
        .as_ref()
        .and_then(|_| crate::input::pointer_poll::poll());
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
    let maximized = ui.input(|i| i.viewport().maximized == Some(true));
    for (k, (rect, dir)) in edges.into_iter().chain(corners).enumerate() {
        // Ids are salted by window: egui interaction state is global
        // across immediate viewports, an unsalted id would let window
        // B's strip replay window A's drag flags.
        let resp = ui.interact(
            rect,
            Id::new("edge_resize").with(win_id).with(k),
            Sense::drag(),
        );
        if resp.drag_started_by(egui::PointerButton::Primary) && !maximized {
            let origin = ui.input(|i| {
                (
                    i.pointer.press_origin(),
                    i.viewport().outer_rect.or(i.viewport().inner_rect),
                )
            });
            // press_origin (not latest_pos): the press frame may coalesce
            // with the first motion event, which would bias the anchor.
            let anchored = origin
                .0
                .and_then(|local| origin.1.map(|rect| rect.min + local.to_vec2()));
            if let (Some(p), Some(rect)) = (anchored, origin.1) {
                *gesture = Some(Gesture {
                    dir,
                    pointer: p,
                    rect,
                });
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) && polled.is_none() {
            if let Some(g) = gesture.filter(|g| g.dir == dir) {
                if let Some(now) = screen_pointer(ui) {
                    let r = resized_rect(&g, now);
                    let ctx = ui.ctx();
                    if moves_origin(g.dir) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(r.min));
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size()));
                }
            }
        }
        resp.on_hover_cursor(cursor_for(dir));
    }
    // While a gesture is live, drive it from the X server's ground
    // truth instead of events: the WM can break the pointer grab under
    // a per-frame resize (seen on openbox), freezing egui's pointer and
    // swallowing the release. Polling needs repaints to keep coming.
    if gesture.is_some() {
        ui.ctx().request_repaint();
        if let (Some(g), Some(p)) = (gesture.as_ref(), polled) {
            let ppp = ui.input(|i| i.pixels_per_point()).max(0.01);
            let now = pos2(p.x / ppp, p.y / ppp);
            let r = resized_rect(g, now);
            let ctx = ui.ctx();
            if moves_origin(g.dir) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(r.min));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(r.size()));
            if !p.primary_down {
                // The button came up (the release event may be gone):
                // end the gesture here, nothing follows the pointer.
                *gesture = None;
            }
            return;
        }
    }
    // Event-driven fallback (no X11: Wayland, tests): apply while the
    // strip drag is live, terminate on release or a hopeless escape.
    let guard = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
    if !guard.0 {
        // The release owns termination: no held primary, nothing follows.
        *gesture = None;
    } else if gesture.is_some() && escaped(win, guard.1) {
        // The pointer outran the window (fast flick past the border).
        // Motion/release can stop being delivered out there, so a live
        // gesture would linger and snap the window on a later re-entry:
        // end it now - the window cannot follow a pointer it cannot see.
        *gesture = None;
    }
}

/// Escape sentinel: how far past the window border (points) a held
/// pointer may report before the gesture is considered lost. Generous
/// enough to absorb one frame of resize lag while following a fast
/// drag, tight enough to catch a flick that left the window behind.
const ESCAPE_MARGIN: f32 = 96.0;

/// True when the tracked pointer is hopelessly outside `win`.
/// A pointer with no position at all (PointerGone) also ends the gesture.
fn escaped(win: Rect, latest: Option<Pos2>) -> bool {
    !latest.is_some_and(|p| win.expand(ESCAPE_MARGIN).contains(p))
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
}
