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
//!
//! A focused TextEdit (rename editor, Settings text fields) publishes its
//! own `o.ime` and never goes through the pane path. Its composition is
//! tracked per window by a latch folded from the same `Event::Ime`
//! stream ([`editor_latch`]); ONLY a live editor composition (or its
//! short post-commit grace) may engage the platform chase. Passing a bare
//! "a text field has focus" there - as this module did before the latch -
//! started the X11 panel chase (16ms repaint loop + X round-trips) for as
//! long as any editor kept focus, with no composition in flight.

use std::time::{Duration, Instant};

use egui::{Context, Rect};
use layout_tree::PaneId;

use crate::state::{Data, WindowUi};

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

/// How long the platform chase stays on after a Commit: the IME server
/// dismisses its candidate panel asynchronously, so its last repositioning
/// can arrive after the commit event itself.
const EDITOR_GRACE: Duration = Duration::from_millis(250);

/// Fold one frame's IME events into the editor composition latch.
///
/// `armed` = previous latch, `preedit` = text of the LAST
/// `ImeEvent::Preedit` this frame (`None` = no preedit event),
/// `commit`/`disabled` = whether such an event arrived, `graced` = the
/// previous Commit's [`EDITOR_GRACE`] deadline is still in the future.
/// Returns `(armed, graced)` for the next frame; the platform chase
/// treats a composition as live while EITHER is true.
///
/// A non-empty preedit arms; an empty preedit is the backend's
/// "composition cancelled" signal and disarms (a still-running
/// post-commit grace survives it - the dismissal it covers may still be in
/// flight). A commit disarms but opens the grace; `Disabled` clears both
/// outright and wins over every other event of the frame.
fn editor_latch(
    armed: bool,
    preedit: Option<&str>,
    commit: bool,
    disabled: bool,
    graced: bool,
) -> (bool, bool) {
    if disabled {
        return (false, false);
    }
    if commit {
        return (false, true);
    }
    match preedit {
        Some("") => (false, graced),
        Some(_) => (true, false),
        None => (armed, graced),
    }
}

/// This frame's editor IME events, collapsed to [`editor_latch`] inputs:
/// the last preedit text and whether a Commit / Disabled event arrived.
#[allow(deprecated)] // ImeEvent::Disabled: deprecated upstream (winit drops it), kept for other backends
fn editor_ime_events(ctx: &Context) -> (Option<String>, bool, bool) {
    ctx.input(|i| {
        let mut preedit: Option<String> = None;
        let mut commit = false;
        let mut disabled = false;
        for ev in &i.events {
            if let egui::Event::Ime(ime) = ev {
                match ime {
                    egui::ImeEvent::Preedit { text, .. } => preedit = Some(text.clone()),
                    egui::ImeEvent::Commit(_) => commit = true,
                    egui::ImeEvent::Disabled => disabled = true,
                    _ => {}
                }
            }
        }
        (preedit, commit, disabled)
    })
}

/// Advance the window's editor latch with this frame's IME events and
/// return whether the platform chase should treat a composition as live
/// (latch armed OR the post-commit grace still running). `now` is injected
/// so the deadline arithmetic stays testable.
fn update_editor_latch(ctx: &Context, w: &mut WindowUi, now: Instant) -> bool {
    let (preedit, commit, disabled) = editor_ime_events(ctx);
    let graced = w.ime_editor_grace.is_some_and(|deadline| now < deadline);
    let (armed, grace) = editor_latch(
        w.ime_editor_composing,
        preedit.as_deref(),
        commit,
        disabled,
        graced,
    );
    w.ime_editor_composing = armed;
    w.ime_editor_grace = if commit {
        Some(now + EDITOR_GRACE) // (re)start the grace window
    } else if grace {
        w.ime_editor_grace // keep the running deadline
    } else {
        None
    };
    armed || grace
}

/// Drop the editor latch and its grace: no editor owns IME any more (the
/// pane path took over, or the focused editor vanished).
pub(crate) fn clear_editor_latch(w: &mut WindowUi) {
    w.ime_editor_composing = false;
    w.ime_editor_grace = None;
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
///
/// The editor path ALSO folds this frame's `Event::Ime` stream into the
/// per-window composition latch ([`WindowUi::ime_editor_composing`] plus
/// its post-commit grace) and passes THAT as `composing`: the platform
/// chase runs only for a genuine editor composition, never for a merely
/// focused text field.
pub fn sync(ctx: &Context, d: &mut Data, idx: usize) {
    if let Some(cursor) = ctx.output(|o| o.ime.map(|ime| ime.cursor_rect)) {
        // A focused TextEdit published its own IME output this frame.
        // Fold this frame's IME events into the composition latch and
        // only engage the platform chase for a live composition (or its
        // post-commit grace); a focused-but-idle editor stays idle.
        let composing = match d.st.windows.get_mut(idx) {
            Some(w) => update_editor_latch(ctx, &mut w.ui, Instant::now()),
            None => false,
        };
        avoid_native_overlap(ctx, cursor, composing);
        return; // a focused rename editor owns IME this frame
    }
    // No editor published IME output this frame: any editor latch (and
    // its grace) is stale and must not restart a chase.
    if let Some(w) = d.st.windows.get_mut(idx) {
        clear_editor_latch(&mut w.ui);
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
