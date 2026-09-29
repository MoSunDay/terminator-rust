//! IME anchoring: pure decisions on the platform IME output, plus the
//! per-window sync that runs at the end of each window render pass.
//!
//! Invariant: while a composition is in flight (`WindowUi.ime` non-empty)
//! the platform IME output stays `Some` with a stable rect and NEVER
//! requests an interrupt. On winit macOS `set_ime_allowed(false)`
//! silently erases the NSView-side marked text without telling the
//! input-method engine (no `unmarkText`), which strands the engine
//! mid-composition and kills the candidate panel until the input
//! source is deactivated/reactivated; on winit X11 the false/true
//! interrupt pair destroys and recreates the XIC and never re-focuses
//! it (same family as the `request_focus` XIM bug). So the anchor
//! LATCHES for the composition's duration (see [`pane_anchor`]) and
//! [`sync`] falls back to the remembered pane + latch on frames whose
//! live anchor is missing (see [`effective_target`]).

use egui::{Context, Rect};
use layout_tree::PaneId;

use crate::state::Data;

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

/// Composition-period anchor latch: `(anchor to use, new latch)`.
///
/// While a composition is in flight the anchor FREEZES on the first
/// live cursor cell it sees (`latched.or(live)`, latched wins so the
/// rect stays bit-stable) and keeps using it even when the live cursor
/// cell transiently vanishes (hidden VT cursor under a TUI, an
/// out-of-grid coordinate during a live resize) or PTY output moves
/// the cursor (a moving anchor spams
/// `invalidateCharacterCoordinates` and makes some input methods
/// bounce or dismiss the candidate panel). With no composition in
/// flight the live rect is used and any stale latch clears.
pub fn pane_anchor(
    composing: bool,
    live: Option<Rect>,
    latched: Option<Rect>,
) -> (Option<Rect>, Option<Rect>) {
    if composing {
        let anchor = latched.or(live);
        (anchor, anchor)
    } else {
        (live, None)
    }
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
/// never focused again. A composition follows its latched anchor; the
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
    if ctx.output(|o| o.ime.is_some()) {
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
    // Assign unconditionally: Some enables IME at the pane cursor, None
    // turns it off (no pane to type into). While composing, the
    // fallback above keeps that Some alive across transient gaps.
    ctx.output_mut(|o| o.ime = out);
    if let Some(w) = d.st.windows.get_mut(idx) {
        w.ui.ime_last_pane = pane;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(8.0, 16.0))
    }

    fn rect_at(y: f32) -> Rect {
        Rect::from_min_size(egui::pos2(10.0, y), egui::vec2(8.0, 16.0))
    }

    #[test]
    fn dead_pane_or_missing_cursor_disables_ime() {
        assert!(platform_ime_output(false, Some(rect()), true).is_none());
        assert!(platform_ime_output(true, None, true).is_none());
    }

    #[test]
    fn live_pane_anchors_terminal_ime() {
        let out = platform_ime_output(true, Some(rect()), true)
            .expect("live pane with cursor enables IME");
        assert_eq!(out.purpose, egui::IMEPurpose::Terminal);
        assert_eq!(out.cursor_rect, rect());
        assert!(!out.should_interrupt_composition);
    }

    #[test]
    fn xim_spot_anchor_sits_at_the_caret_baseline() {
        // X11: rect.min = bottom-left of the caret cell (the XIM spot
        // convention shared with xterm/GTK), size carried through.
        let anchored = anchor_rect(rect(), true);
        assert_eq!(anchored.min, egui::pos2(10.0, 36.0));
        assert_eq!(anchored.size(), rect().size());
        // Full-rect backends (macOS, Wayland) keep the true caret cell.
        assert_eq!(anchor_rect(rect(), false), rect());
        let out =
            platform_ime_output(true, Some(rect()), true).expect("spot anchor keeps IME alive");
        assert_eq!(out.rect.min, egui::pos2(10.0, 36.0));
        assert_eq!(out.cursor_rect, rect());
    }

    #[test]
    fn ime_output_never_interrupts_composition() {
        // Even a pane switch mid-composition must not ask the platform
        // to interrupt: egui-winit implements the flag as a
        // set_ime_allowed(false)/(true) pair, which erases macOS marked
        // text and rebuilds an unfocused XIC on X11.
        let out = platform_ime_output(true, Some(rect()), false).expect("anchored");
        assert!(!out.should_interrupt_composition);
    }

    #[test]
    fn composition_latches_the_first_live_anchor() {
        // Composing with no latch yet: the live rect latches.
        let (anchor, latch) = pane_anchor(true, Some(rect_at(20.0)), None);
        assert_eq!(anchor, Some(rect_at(20.0)));
        assert_eq!(latch, Some(rect_at(20.0)));
        // Later frames: the latch wins over a moved live cursor
        // (stable rect, no invalidateCharacterCoordinates spam).
        let (anchor, latch) = pane_anchor(true, Some(rect_at(40.0)), Some(rect_at(20.0)));
        assert_eq!(anchor, Some(rect_at(20.0)));
        assert_eq!(latch, Some(rect_at(20.0)));
        // A transiently missing live cursor keeps the latched anchor.
        let (anchor, latch) = pane_anchor(true, None, Some(rect_at(20.0)));
        assert_eq!(anchor, Some(rect_at(20.0)));
        assert_eq!(latch, Some(rect_at(20.0)));
    }

    #[test]
    fn no_composition_uses_live_anchor_and_clears_latch() {
        let (anchor, latch) = pane_anchor(false, Some(rect_at(40.0)), Some(rect_at(20.0)));
        assert_eq!(anchor, Some(rect_at(40.0)));
        assert_eq!(latch, None, "stale latch clears once composing ends");
        // Composing with neither latch nor live: nothing to anchor on.
        let (anchor, latch) = pane_anchor(true, None, None);
        assert_eq!(anchor, None);
        assert_eq!(latch, None);
    }

    #[test]
    fn composition_falls_back_to_remembered_pane_and_latch() {
        let pane = 7u64;
        // Live anchor missing while composing: remembered pane + latch.
        assert_eq!(
            effective_target(true, None, None, Some(pane), Some(rect())),
            (Some(pane), Some(rect()))
        );
        // Live anchor present: passes through unchanged (latch already
        // applied by screen.rs via pane_anchor).
        assert_eq!(
            effective_target(true, Some(pane), Some(rect()), Some(8), None),
            (Some(pane), Some(rect()))
        );
        // Not composing: live values pass through, even when None.
        assert_eq!(
            effective_target(false, None, None, Some(pane), Some(rect())),
            (None, None)
        );
    }
}
