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
    }
}

fn send_keypress(
    pane: PaneId,
    sess: &mut Session,
    key: GKey,
    mods: GMods,
    utf8: Option<String>,
) {
    let res = vtask::send_key(sess, |ev| {
        ev.set_action(GAction::Press);
        ev.set_key(key);
        ev.set_mods(mods);
        ev.set_utf8(utf8);
    });
    if let Err(e) = res {
        warn!("send key to pane {pane}: {e}");
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
    if ctx.egui_wants_keyboard_input() {
        return; // a text field has focus; let it keep the keys
    }
    let events = ctx.input(|i| i.events.clone());
    let alt_chars = alt_keyed_chars(&events);
    let tab = st.tree.active_tab.min(st.tree.tabs.len().saturating_sub(1));
    let focused = st.tree.tabs.get(tab).map(|t| t.focused);
    for ev in events {
        match ev {
            Event::Paste(text) => {
                if let Some(p) = focused {
                    if let Some(s) = sess.map.get_mut(&p) {
                        if let Err(e) = vtask::paste(s, &text) {
                            warn!("paste to pane {p}: {e}");
                        }
                    }
                }
            }
            Event::Copy | Event::Cut => {} // TODO: copy selection once selection exists
            Event::Text(t) => {
                // Alt+letter also emits a Text event on X11; the encoded Key
                // event already carried the combo, so drop the raw char.
                if t.chars().next().is_some_and(|c| alt_chars.contains(&c.to_ascii_lowercase())) {
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
