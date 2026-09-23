//! Border strip widgets and live-gesture driving: registers the eight
//! invisible strips as the LAST widgets of a window pass, then follows
//! the pointer each frame - X11 polled ground truth when available,
//! the event path (with the win.expand escape guard) as fallback.

use egui::{pos2, Id, Pos2, Rect, Sense, Ui};

use super::gesture::{moves_origin, resized_rect, Gesture};
use super::hit::{cursor_for, CORNER, EDGE};

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
            Sense::DRAG,
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
// resize-private (was module-private pre-split): reached by `tests`
// through mod.rs's cfg(test) import.
pub(super) fn escaped(win: Rect, latest: Option<Pos2>) -> bool {
    !latest.is_some_and(|p| win.expand(ESCAPE_MARGIN).contains(p))
}
