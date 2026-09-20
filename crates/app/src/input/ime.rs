//! IME anchoring: pure decision on the platform IME output, plus the
//! per-window sync that runs at the end of each window render pass.

use egui::{Context, Rect};
use layout_tree::PaneId;

use crate::state::Data;

/// Decide the platform IME output for a terminal pane.
///
/// None when a text editor owns the keyboard, the pane is dead, or no
/// cursor rect is known; otherwise the IME popup anchors at the pane
/// cursor cell (purpose = Terminal) and an anchor-pane change asks the
/// platform to interrupt any in-flight composition.
pub fn platform_ime_output(
    editor_focused: bool,
    pane_alive: bool,
    cursor: Option<Rect>,
    pane_changed: bool,
) -> Option<egui::output::IMEOutput> {
    if editor_focused || !pane_alive {
        return None;
    }
    Some(egui::output::IMEOutput {
        purpose: egui::IMEPurpose::Terminal,
        rect: cursor?,
        cursor_rect: cursor?,
        should_interrupt_composition: pane_changed,
    })
}

/// Is `pane`'s session alive (present and not exited)?
fn pane_alive(pane: PaneId, d: &Data) -> bool {
    d.sess.map.get(&pane).is_some_and(|s| s.exit.is_none())
}

/// Sync this window's IME state into the platform output. Call at the END
/// of the window render pass, after screen() refreshed `ime_cursor` /
/// `ime_pane`. While a TextEdit owns the keyboard (rename editors) its
/// own IME output must stand: return without touching `o.ime`.
pub fn sync(ctx: &Context, d: &mut Data, idx: usize) {
    if ctx.egui_wants_keyboard_input() {
        return; // a text field owns IME this frame
    }
    let Some(w) = d.st.windows.get(idx) else {
        return;
    };
    let pane = w.ui.ime_pane;
    let cursor = w.ui.ime_cursor;
    let alive = pane.is_some_and(|p| pane_alive(p, d));
    let changed = pane != w.ui.ime_last_pane;
    let out = platform_ime_output(false, alive, cursor, changed);
    // Assign unconditionally: Some enables IME at the pane cursor, None
    // turns it off (no editor owns it this frame).
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

    #[test]
    fn editor_focus_disables_ime() {
        assert!(platform_ime_output(true, true, Some(rect()), false).is_none());
    }

    #[test]
    fn dead_pane_or_missing_cursor_disables_ime() {
        assert!(platform_ime_output(false, false, Some(rect()), false).is_none());
        assert!(platform_ime_output(false, true, None, false).is_none());
    }

    #[test]
    fn live_pane_anchors_terminal_ime() {
        let out = platform_ime_output(false, true, Some(rect()), false)
            .expect("live pane with cursor enables IME");
        assert_eq!(out.purpose, egui::IMEPurpose::Terminal);
        assert_eq!(out.rect, rect());
        assert_eq!(out.cursor_rect, rect());
        assert!(!out.should_interrupt_composition);
    }

    #[test]
    fn pane_change_interrupts_composition_once() {
        let out = platform_ime_output(false, true, Some(rect()), true)
            .expect("pane change still anchors IME");
        assert!(out.should_interrupt_composition);
        // Same pane again: no interrupt (decision is a pure function of
        // the changed flag; sync computes it from ime_last_pane).
        let out =
            platform_ime_output(false, true, Some(rect()), false).expect("same pane keeps IME");
        assert!(!out.should_interrupt_composition);
    }
}
