//! egui key/text events -> terminal input, after global shortcut routing.

use egui::{Context, Event};
use layout_tree::PaneId;
use libghostty_vt::key::{Action as GAction, Key as GKey, Mods as GMods};
use log::warn;
use vt_pane::{task as vtask, Session};

use crate::actions;
use crate::input::keymap::{
    alt_keyed_chars, ghostty_key, ghostty_mods, is_command_key, key_char, produces_text, to_skey,
    to_smods,
};
use crate::input::scroll;
use crate::session_map::SessionMap;
use crate::state::{self, Action, AppState, UiState};

/// Ghostty key for a plain character (letters, digits); None otherwise.
pub fn key_from_char(c: char) -> Option<GKey> {
    let g = match c {
        'a' | 'A' => GKey::A,
        'b' | 'B' => GKey::B,
        'c' | 'C' => GKey::C,
        'd' | 'D' => GKey::D,
        'e' | 'E' => GKey::E,
        'f' | 'F' => GKey::F,
        'g' | 'G' => GKey::G,
        'h' | 'H' => GKey::H,
        'i' | 'I' => GKey::I,
        'j' | 'J' => GKey::J,
        'k' | 'K' => GKey::K,
        'l' | 'L' => GKey::L,
        'm' | 'M' => GKey::M,
        'n' | 'N' => GKey::N,
        'o' | 'O' => GKey::O,
        'p' | 'P' => GKey::P,
        'q' | 'Q' => GKey::Q,
        'r' | 'R' => GKey::R,
        's' | 'S' => GKey::S,
        't' | 'T' => GKey::T,
        'u' | 'U' => GKey::U,
        'v' | 'V' => GKey::V,
        'w' | 'W' => GKey::W,
        'x' | 'X' => GKey::X,
        'y' | 'Y' => GKey::Y,
        'z' | 'Z' => GKey::Z,
        '0' => GKey::Digit0,
        '1' => GKey::Digit1,
        '2' => GKey::Digit2,
        '3' => GKey::Digit3,
        '4' => GKey::Digit4,
        '5' => GKey::Digit5,
        '6' => GKey::Digit6,
        '7' => GKey::Digit7,
        '8' => GKey::Digit8,
        '9' => GKey::Digit9,
        _ => return None,
    };
    Some(g)
}

/// Send one plain character through the key encoder (Text path).
fn send_char(pane: PaneId, sess: &mut Session, c: char) {
    let key = key_from_char(c);
    let res = vtask::send_key(sess, |ev| {
        ev.set_action(GAction::Press);
        if let Some(k) = key {
            ev.set_key(k);
        }
        ev.set_mods(GMods::empty());
        ev.set_utf8(Some(c.to_string()));
    });
    if let Err(e) = res {
        warn!("send char to pane {pane}: {e}");
    } else {
        // Typing snaps the viewport back to the live area (desktop habit).
        vt_pane::mouse::follow_output(sess);
    }
}

fn send_keypress(pane: PaneId, sess: &mut Session, key: GKey, mods: GMods, utf8: Option<String>) {
    let res = vtask::send_key(sess, |ev| {
        ev.set_action(GAction::Press);
        ev.set_key(key);
        ev.set_mods(mods);
        ev.set_utf8(utf8);
    });
    if let Err(e) = res {
        warn!("send key to pane {pane}: {e}");
    } else {
        vt_pane::mouse::follow_output(sess);
    }
}

/// Best-effort clipboard read for the Ctrl+Shift+V paste shortcut; empty
/// on any failure (egui 0.36 exposes no clipboard-read on Context).
fn read_clipboard() -> String {
    arboard::Clipboard::new()
        .and_then(|mut c| c.get_text())
        .unwrap_or_default()
}

/// Frame-level input dispatch: shortcuts first, then terminal input.
pub fn handle(
    ctx: &Context,
    st: &mut AppState,
    sess: &mut SessionMap,
    ui: &mut UiState,
    dirty: &mut bool,
) {
    // Per-event mods start from the state at the START of this batch: the
    // previous frame's end, captured BEFORE the text-field early return so
    // it stays current. `i.modifiers` is the POST-batch aggregate and is
    // never used per-event; mods_at below advances over the batch's
    // ModifiersChanged marks instead. Why it matters: a fast bare Ctrl+C is
    // folded by egui-winit into Event::Copy with no Key event, and when the
    // ctrl-down marks landed in an earlier frame, trusting `i.modifiers`
    // (or a stale seed) makes the folded Copy look modifier-less and
    // silently reroutes SIGINT to the clipboard path.
    let mods_at_start = ui.mods_frame_end;
    ui.mods_frame_end = ctx.input(|i| i.modifiers);
    if ctx.egui_wants_keyboard_input() {
        return; // a text field has focus; let it keep the keys
    }
    let events = ctx.input(|i| i.events.clone());
    // advance over ModifiersChanged marks; see the seeding note above
    let mut mods_at = mods_at_start;
    let alt_chars = alt_keyed_chars(&events);
    let tab = st.tree.active_tab.min(st.tree.tabs.len().saturating_sub(1));
    let focused = st.tree.tabs.get(tab).map(|t| t.focused);
    for ev in events {
        if let Event::ModifiersChanged(m) = ev {
            mods_at = m;
            continue;
        }
        match ev {
            Event::Paste(text) => {
                // Bare Ctrl+V is quoted-insert in the child, not a paste
                // (Ctrl+Shift+V is the terminal paste shortcut).
                if mods_at.ctrl && !mods_at.shift {
                    if let Some(p) = focused {
                        if let Some(s) = sess.map.get_mut(&p) {
                            if let Some(gk) = key_from_char('v') {
                                send_keypress(p, s, gk, ghostty_mods(&mods_at), None);
                            }
                        }
                    }
                } else if let Some(p) = focused {
                    if let Some(s) = sess.map.get_mut(&p) {
                        if let Err(e) = vtask::paste(s, &text) {
                            warn!("paste to pane {p}: {e}");
                        } else {
                            vt_pane::mouse::follow_output(s);
                        }
                    }
                }
            }
            Event::Copy | Event::Cut => {
                // egui-winit folds ctrl/cmd+C/X into clipboard events; bare
                // Ctrl combos must still reach the child (SIGINT, ^X), so
                // only Shift / dedicated-key / macOS-Cmd forms act on the
                // clipboard.
                if mods_at.ctrl && !mods_at.shift {
                    let c = if matches!(ev, Event::Cut) { 'x' } else { 'c' };
                    if let Some(p) = focused {
                        if let Some(s) = sess.map.get_mut(&p) {
                            if let Some(gk) = key_from_char(c) {
                                send_keypress(p, s, gk, ghostty_mods(&mods_at), None);
                            }
                        }
                    }
                } else {
                    actions::copy_focused(st, sess);
                }
            }
            Event::Text(t) => {
                // Alt+letter also emits a Text event on X11; the encoded Key
                // event already carried the combo, so drop the raw char.
                if t.chars()
                    .next()
                    .is_some_and(|c| alt_chars.contains(&c.to_ascii_lowercase()))
                {
                    continue;
                }
                if let Some(p) = focused {
                    if let Some(s) = sess.map.get_mut(&p) {
                        for c in t.chars() {
                            send_char(p, s, c);
                        }
                    }
                }
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if let Some(sk) = to_skey(key) {
                    if let Some(action) = state::route_shortcut(to_smods(&modifiers), sk) {
                        if action == Action::Paste {
                            // Paste needs clipboard access, which lives here.
                            let text = read_clipboard();
                            if !text.is_empty() {
                                if let Some(p) = focused {
                                    if let Some(s) = sess.map.get_mut(&p) {
                                        if let Err(e) = vtask::paste(s, &text) {
                                            warn!("paste to pane {p}: {e}");
                                        } else {
                                            vt_pane::mouse::follow_output(s);
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        actions::apply_action(st, sess, ui, action, dirty);
                        continue;
                    }
                }
                // Shift+PageUp/PageDown/Home/End review local scrollback
                // (GNOME Terminal habit); all other combos reach the child.
                if let Some(p) = focused {
                    if let Some(s) = sess.map.get_mut(&p) {
                        if scroll::handle_key(s, key, &modifiers) {
                            continue;
                        }
                    }
                }
                if !is_command_key(&modifiers) && produces_text(key) {
                    continue; // the matching Event::Text carries the character
                }
                let Some(gk) = ghostty_key(key) else { continue };
                let mods = ghostty_mods(&modifiers);
                // The legacy encoder drops Alt+printable unless utf8 is set.
                let utf8 = if mods.intersects(GMods::ALT) {
                    key_char(key, modifiers.shift).map(String::from)
                } else {
                    None
                };
                if let Some(p) = focused {
                    if let Some(s) = sess.map.get_mut(&p) {
                        send_keypress(p, s, gk, mods, utf8);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_map::session_map;
    use crate::state::{fresh_state, ui_state};
    use egui::{Modifiers, RawInput};
    use vt_pane::Session;
    use vt_pane::SessionOpts;

    /// Pane 1 focused, running `stty -isig; echo READY; cat` on a real
    /// pty: with ISIG off, ^C reaches cat as a plain byte and the tty
    /// echoes it in caret notation, so the grid proves what the pty got.
    /// Skips when no pty is available.
    fn harness() -> Option<(AppState, SessionMap, UiState)> {
        let opts = SessionOpts::command(
            20,
            5,
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "stty -isig; echo READY; cat".to_string(),
            ],
        );
        match vtask::spawn_session(&opts) {
            Ok(s) => {
                let mut sess = session_map();
                sess.map.insert(1, s);
                Some((fresh_state(), sess, ui_state()))
            }
            Err(e) => {
                eprintln!("skip: {e}");
                None
            }
        }
    }

    /// One synthetic egui pass carrying the event + modifiers, then the
    /// normal frame-level dispatch.
    fn dispatch(
        ctx: &Context,
        mods: Modifiers,
        ev: Event,
        st: &mut AppState,
        sess: &mut SessionMap,
        ui: &mut UiState,
    ) {
        // egui 0.36 has no modifiers field on RawInput; modifier state is
        // carried by ModifiersChanged events (as egui-winit delivers them).
        let input = RawInput {
            events: vec![Event::ModifiersChanged(mods), ev],
            ..Default::default()
        };
        ctx.begin_pass(input);
        let mut dirty = false;
        handle(ctx, st, sess, ui, &mut dirty);
        let mut out = ctx.end_pass();
        // Nothing paints this context: discard the first-pass font texture
        // delta instead of tripping its drop-time unapplied-delta assert.
        out.textures_delta.clear();
    }

    fn grid_text(s: &mut Session) -> String {
        let _ = vtask::pump(s);
        match vtask::frame(s) {
            Ok(f) => f.cells.iter().flatten().map(|c| c.text.as_str()).collect(),
            Err(_) => String::new(),
        }
    }

    /// Pump until the grid shows the needle (stty settled, echo alive).
    fn wait_grid(s: &mut Session, needle: &str) -> bool {
        for _ in 0..100 {
            if grid_text(s).contains(needle) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn bare_ctrl_copy_sends_interrupt_to_pty() {
        let Some((mut st, mut sess, mut ui)) = harness() else {
            return;
        };
        let ctx = Context::default();
        {
            let Some(s) = sess.map.get_mut(&1) else {
                return;
            };
            assert!(wait_grid(s, "READY"), "session did not start echoing");
        }
        dispatch(
            &ctx,
            Modifiers::CTRL,
            Event::Copy,
            &mut st,
            &mut sess,
            &mut ui,
        );
        let Some(s) = sess.map.get_mut(&1) else {
            return;
        };
        // The tty driver echoes the raw \x03 in caret notation.
        assert!(wait_grid(s, "^C"), "pty never saw the ^C byte");
    }

    #[test]
    fn ctrl_shift_copy_stays_on_clipboard() {
        let Some((mut st, mut sess, mut ui)) = harness() else {
            return;
        };
        let ctx = Context::default();
        {
            let Some(s) = sess.map.get_mut(&1) else {
                return;
            };
            assert!(wait_grid(s, "READY"), "session did not start echoing");
        }
        dispatch(
            &ctx,
            Modifiers::CTRL | Modifiers::SHIFT,
            Event::Copy,
            &mut st,
            &mut sess,
            &mut ui,
        );
        // Empty selection -> clipboard path is a no-op; nothing may reach
        // the pty and cat must still be alive.
        let Some(s) = sess.map.get_mut(&1) else {
            return;
        };
        for _ in 0..30 {
            let text = grid_text(s);
            assert!(
                !text.contains("^C"),
                "ctrl+shift leaked ^C to the pty: {text:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(s.exit.is_none(), "cat died without SIGINT");
    }
}
