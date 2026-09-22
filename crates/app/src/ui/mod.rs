//! Chrome UI: tab bar, pane headers and the inspector window.

pub mod chrome;
pub mod fonts;
pub mod inspector;
pub mod pane_header;
pub mod style;
pub mod tabs;
pub mod tabs_widgets;

/// Minimum pointer travel (points) from the press origin before a
/// bare-chrome / lone-pane-header press becomes a WM window-move gesture.
pub(crate) const WINDOW_DRAG_MIN_PX: f32 = 8.0;

/// Gate ViewportCommand::StartDrag on REAL pointer travel. egui's own drag
/// threshold can be defeated by coalesced press+motion frames (slow WM or
/// busy desktop), and some WMs (xfwm4 focus-delay configs) treat the
/// _NET_WM_MOVERESIZE grab as a gesture that eats the click-to-focus of
/// the initiating press - the window then never activates on click.
/// Micro-drift clicks stay plain clicks so the WM's click-to-focus keeps
/// working; the latch sends StartDrag exactly once per gesture and re-arms
/// when the button comes up.
pub(crate) fn arm_window_drag(
    ctx: &egui::Context,
    armed: &mut bool,
    travel: &mut f32,
    dragging: bool,
    drag_delta_len: f32,
    button_down: bool,
    fresh_press: bool,
) {
    if fresh_press {
        // A new press begins a new gesture: clear any latch residue from a
        // gesture whose release was swallowed by the WM move-grab.
        *armed = false;
        *travel = 0.0;
    }
    if dragging {
        *travel += drag_delta_len;
    }
    if button_down {
        if !*armed && *travel >= WINDOW_DRAG_MIN_PX {
            *armed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
    } else {
        *armed = false;
        *travel = 0.0;
    }
}
