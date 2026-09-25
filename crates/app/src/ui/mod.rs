//! Chrome UI: tab bar, pane headers and the inspector window.

pub mod chrome;
pub mod close_dialog;
pub mod fonts;
pub mod inspector;
pub mod pane_header;
pub mod style;
pub mod tabs;
pub mod tabs_widgets;
pub mod xdrag;

/// Minimum pointer travel (points) from the press origin before a
/// bare-chrome / lone-pane-header press becomes a WM window-move gesture.
pub(crate) const WINDOW_DRAG_MIN_PX: f32 = 8.0;

/// Give a freshly-shown rename editor the keyboard WITHOUT the IME
/// interrupt that `Response::request_focus` triggers.
///
/// `request_focus` calls `Memory::interrupt_ime`, and egui-winit turns the
/// flag into `set_ime_allowed(false)` + `set_ime_allowed(true)`. On winit
/// X11 that pair DESTROYS and recreates the XIM input context, and a fresh
/// context is never `XSetICFocus`-ed again (winit only focuses the IC from
/// a window FocusIn), so the next keystrokes bypass the input method and
/// land as raw latin: double-click a chip, type pinyin, get `hanzi`
/// instead of 汉字. The same pair costs the first key after the editor
/// closes (Enter commits, the pane's IME comes back only on the NEXT
/// frame).
///
/// egui's Tab-navigation path hands focus over without interrupting:
/// `Focus::interested_in_focus` focuses the first focus-interested widget
/// of the frame while `focus_direction == Next`. Call this BEFORE adding
/// the editor, then the editor is that widget. Rename editors are the only
/// focusable widgets the chrome draws, and their ids are unique, so the
/// focus cannot land anywhere else - and the input context survives.
pub(crate) fn focus_rename_editor(ctx: &egui::Context, id: egui::Id) {
    if ctx.memory(|m| m.has_focus(id)) {
        return; // already focused - a re-request would push focus past it
    }
    ctx.memory_mut(|m| {
        if let Some(other) = m.focused() {
            // Steal the keyboard from whatever holds it (a Settings panel
            // field): `request_focus` would have, and only the interrupt
            // is dropped here, never the handover.
            m.surrender_focus(other);
        }
        m.move_focus(egui::FocusDirection::Next);
    });
}

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
