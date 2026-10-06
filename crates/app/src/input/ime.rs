//! IME anchoring: pure decisions on the platform IME output, plus the
//! per-window sync that runs at the end of each window render pass.
//!
//! Invariant: while a composition is in flight (`WindowUi.ime` non-empty)
//! the platform IME output stays `Some` with the current or last known cursor and NEVER
//! requests an interrupt. On winit macOS `set_ime_allowed(false)`
//! silently erases the NSView-side marked text without telling the
//! input-method engine (no `unmarkText`), which strands the engine
//! mid-composition and kills the candidate panel until the input
//! source is deactivated/reactivated; on winit X11 the false/true
//! interrupt pair destroys and recreates the XIC and never re-focuses
//! it (same family as the `request_focus` XIM bug). The anchor
//! follows live cursor updates (see [`pane_anchor`]) and
//! [`sync`] falls back to the remembered pane + latch on frames whose
//! live anchor is missing (see [`effective_target`]).

use egui::{Context, Rect};
use layout_tree::PaneId;

use crate::state::Data;

#[cfg(any(target_os = "linux", test))]
mod popup;
#[cfg(target_os = "linux")]
mod x11;

fn avoid_native_overlap(ctx: &Context, cursor: Rect, composing: bool) {
    #[cfg(target_os = "linux")]
    x11::sync(ctx, cursor, composing);
    #[cfg(not(target_os = "linux"))]
    let _ = (ctx, cursor, composing);
}

/// Platform IME popup anchor derived from the pane cursor cell.
///
/// egui-winit forwards `rect` to `Window::set_ime_cursor_area` and the
/// backends consume it differently: X11 reads ONLY `rect.min` as the raw
/// XIM spot point (the size is ignored), and the X11 convention (xterm,
/// GTK) puts that spot at the caret baseline - the BOTTOM-left of the
/// cell - so the candidate window floats below the composing line like
/// every other app. macOS and Wayland consume the full rect and anchor
/// below it; they keep the true caret cell.
pub fn anchor_rect(cell: Rect, spot_at_baseline: bool) -> Rect {
    if spot_at_baseline {
        Rect::from_min_size(egui::pos2(cell.left(), cell.bottom()), cell.size())
    } else {
        cell
    }
}

/// Does this backend treat the IME rect min as an XIM spot point?
/// Linux/X11 yes (winit ignores the rect size); Wayland and every other
/// platform consume the whole rect.
fn spot_at_baseline() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none()
}

/// Follow the terminal's current cursor, remembering it across hidden-cursor
/// or resize transition frames while composing. A commit only writes to the
/// PTY: its echo can arrive AFTER the next Preedit, so freezing the first
/// cursor would paint the next word over that late echo indefinitely.
///
/// Keeping output enabled and interrupt-free protects the native composition;
/// a valid position update does not disable IME or erase marked text. The
/// egui-winit backend already updates native coordinates on every input event,
/// even when the rectangle is unchanged.
pub fn pane_anchor(
    composing: bool,
    live: Option<Rect>,
    remembered: Option<Rect>,
) -> (Option<Rect>, Option<Rect>) {
    let anchor = if composing { live.or(remembered) } else { live };
    (anchor, anchor)
}

/// Effective `(pane, rect)` for the platform output, with the
/// composition-period fallback applied: while composing, a frame whose
/// live anchor is missing (dead pane, empty tree, dead-session frame)
/// falls back to the remembered pane + latched rect so `o.ime` never
/// flips to None mid-composition. Outside a composition the live
/// values pass straight through (None output = IME off, no pane to
/// type into).
pub fn effective_target(
    composing: bool,
    live_pane: Option<PaneId>,
    live_cursor: Option<Rect>,
    last_pane: Option<PaneId>,
    latched: Option<Rect>,
) -> (Option<PaneId>, Option<Rect>) {
    if composing && live_pane.is_none() {
        (last_pane, latched)
    } else {
        (live_pane, live_cursor)
    }
}

/// Decide the platform IME output for a terminal pane.
///
/// None when the pane is dead or no cursor rect is known; otherwise
/// the IME popup anchors at the pane cursor cell (purpose = Terminal).
/// `should_interrupt_composition` is ALWAYS false: winit has no real
/// interrupt API and egui-winit emulates the flag with a
/// `set_ime_allowed(false); (true)` pair, which on macOS silently
/// erases the client marked text (stranding the engine's composition
/// and killing the candidate panel) and on X11 rebuilds an XIC that is
/// never focused again. A composition follows the current or last known cursor; the
/// commit lands on whatever pane is focused when it arrives (see
/// keyboard.rs), which is the correct terminal behaviour.
pub fn platform_ime_output(
    pane_alive: bool,
    cursor: Option<Rect>,
    spot_at_baseline: bool,
) -> Option<egui::output::IMEOutput> {
    if !pane_alive {
        return None;
    }
    Some(egui::output::IMEOutput {
        purpose: egui::IMEPurpose::Terminal,
        rect: anchor_rect(cursor?, spot_at_baseline),
        cursor_rect: cursor?,
        should_interrupt_composition: false,
    })
}

/// Is `pane`'s session alive (present and not exited)?
fn pane_alive(pane: PaneId, d: &Data) -> bool {
    d.sess.map.get(&pane).is_some_and(|s| s.exit.is_none())
}

/// Sync this window's IME state into the platform output. Call at the END
/// of the window render pass, after screen() refreshed `ime_cursor` /
/// `ime_pane` / `ime_anchor`. While a TextEdit owns the keyboard (rename
/// editors) its own IME output must stand: return without touching
/// `o.ime`.
///
/// The test is "did anyone publish an IME output", NOT "is a text field
/// focused": the rename editors publish nothing on the frame they gain
/// focus from a stale state and on the Enter frame that surrenders focus,
/// and publishing None there makes egui-winit disable+re-enable IME. On
/// winit X11 that destroys+recreates the XIM input context, which is never
/// re-focused, so the next keystroke bypasses the input method - the pane's
/// first key after a rename lands as raw latin. Keeping the pane anchor
/// alive across those single frames never interrupts anything.
pub fn sync(ctx: &Context, d: &mut Data, idx: usize) {
    if let Some(cursor) = ctx.output(|o| o.ime.map(|ime| ime.cursor_rect)) {
        avoid_native_overlap(ctx, cursor, true);
        return; // a focused rename editor owns IME this frame
    }
    let Some(w) = d.st.windows.get(idx) else {
        return;
    };
    let composing = w.ui.ime.as_deref().is_some_and(|t| !t.is_empty());
    let (pane, cursor) = effective_target(
        composing,
        w.ui.ime_pane,
        w.ui.ime_cursor,
        w.ui.ime_last_pane,
        w.ui.ime_anchor,
    );
    let alive = pane.is_some_and(|p| pane_alive(p, d));
    let out = platform_ime_output(alive, cursor, spot_at_baseline());
    if let Some(out) = &out {
        avoid_native_overlap(ctx, out.cursor_rect, composing);
    }
    // Assign unconditionally: Some enables IME at the pane cursor, None
    // turns it off (no pane to type into). While composing, the
    // fallback above keeps that Some alive across transient gaps.
    ctx.output_mut(|o| o.ime = out);
    if let Some(w) = d.st.windows.get_mut(idx) {
        w.ui.ime_last_pane = pane;
    }
}

#[cfg(test)]
#[path = "ime/tests.rs"]
mod tests;
