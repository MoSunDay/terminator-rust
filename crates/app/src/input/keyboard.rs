//! egui key/text events -> terminal input, after global shortcut routing.

use egui::{Context, Event, Key, Modifiers};
use layout_tree::PaneId;
use libghostty_vt::key::{Action as GAction, Key as GKey, Mods as GMods};
use log::warn;
use vt_pane::{task as vtask, Session};

use crate::actions;
use crate::session_map::SessionMap;
use crate::state::{self, AppState, SKey, SMods, UiState};

/// egui modifiers -> ghostty Mods.
pub fn ghostty_mods(m: &Modifiers) -> GMods {
    let mut g = GMods::empty();
    if m.shift {
        g |= GMods::SHIFT;
    }
    if m.ctrl {
        g |= GMods::CTRL;
    }
    if m.alt {
        g |= GMods::ALT;
    }
    if m.mac_cmd {
        g |= GMods::SUPER;
    }
    g
}

/// Map an egui logical key to a ghostty key (None = unmapped/ignore).
pub fn ghostty_key(k: Key) -> Option<GKey> {
    let g = match k {
        Key::A => GKey::A,
        Key::B => GKey::B,
        Key::C => GKey::C,
        Key::D => GKey::D,
        Key::E => GKey::E,
        Key::F => GKey::F,
        Key::G => GKey::G,
        Key::H => GKey::H,
        Key::I => GKey::I,
        Key::J => GKey::J,
        Key::K => GKey::K,
        Key::L => GKey::L,
        Key::M => GKey::M,
        Key::N => GKey::N,
        Key::O => GKey::O,
        Key::P => GKey::P,
        Key::Q => GKey::Q,
        Key::R => GKey::R,
        Key::S => GKey::S,
        Key::T => GKey::T,
        Key::U => GKey::U,
        Key::V => GKey::V,
        Key::W => GKey::W,
        Key::X => GKey::X,
        Key::Y => GKey::Y,
        Key::Z => GKey::Z,
        Key::Num0 => GKey::Digit0,
        Key::Num1 => GKey::Digit1,
        Key::Num2 => GKey::Digit2,
        Key::Num3 => GKey::Digit3,
        Key::Num4 => GKey::Digit4,
        Key::Num5 => GKey::Digit5,
        Key::Num6 => GKey::Digit6,
        Key::Num7 => GKey::Digit7,
        Key::Num8 => GKey::Digit8,
        Key::Num9 => GKey::Digit9,
        Key::Space => GKey::Space,
        Key::Tab => GKey::Tab,
        Key::Enter => GKey::Enter,
        Key::Backspace => GKey::Backspace,
        Key::Escape => GKey::Escape,
        Key::Insert => GKey::Insert,
        Key::Delete => GKey::Delete,
        Key::Home => GKey::Home,
        Key::End => GKey::End,
        Key::PageUp => GKey::PageUp,
        Key::PageDown => GKey::PageDown,
        Key::ArrowUp => GKey::ArrowUp,
        Key::ArrowDown => GKey::ArrowDown,
        Key::ArrowLeft => GKey::ArrowLeft,
        Key::ArrowRight => GKey::ArrowRight,
        Key::Comma => GKey::Comma,
        Key::Minus => GKey::Minus,
        Key::Period => GKey::Period,
        Key::Slash => GKey::Slash,
        Key::Semicolon => GKey::Semicolon,
        Key::Quote => GKey::Quote,
        Key::Backslash => GKey::Backslash,
        Key::Backtick => GKey::Backquote,
        Key::OpenBracket => GKey::BracketLeft,
        Key::CloseBracket => GKey::BracketRight,
        Key::Equals => GKey::Equal,
        Key::F1 => GKey::F1,
        Key::F2 => GKey::F2,
        Key::F3 => GKey::F3,
        Key::F4 => GKey::F4,
        Key::F5 => GKey::F5,
        Key::F6 => GKey::F6,
        Key::F7 => GKey::F7,
        Key::F8 => GKey::F8,
        Key::F9 => GKey::F9,
        Key::F10 => GKey::F10,
        Key::F11 => GKey::F11,
        Key::F12 => GKey::F12,
        Key::Copy => GKey::Copy,
        Key::Cut => GKey::Cut,
        Key::Paste => GKey::Paste,
        _ => return None,
    };
    Some(g)
}

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

/// egui keys whose input arrives (also) as an `Event::Text`, so the KeyDown
/// must be skipped to avoid double input. With ctrl/command held egui-winit
/// emits no Text, so those still go through the Key path.
pub fn produces_text(k: Key) -> bool {
    matches!(
        k,
        Key::A
            | Key::B
            | Key::C
            | Key::D
            | Key::E
            | Key::F
            | Key::G
            | Key::H
            | Key::I
            | Key::J
            | Key::K
            | Key::L
            | Key::M
            | Key::N
            | Key::O
            | Key::P
            | Key::Q
            | Key::R
            | Key::S
            | Key::T
            | Key::U
            | Key::V
            | Key::W
            | Key::X
            | Key::Y
            | Key::Z
            | Key::Num0
            | Key::Num1
            | Key::Num2
            | Key::Num3
            | Key::Num4
            | Key::Num5
            | Key::Num6
            | Key::Num7
            | Key::Num8
            | Key::Num9
            | Key::Space
            | Key::Comma
            | Key::Minus
            | Key::Period
            | Key::Slash
            | Key::Semicolon
            | Key::Quote
            | Key::Backslash
            | Key::Backtick
            | Key::OpenBracket
            | Key::CloseBracket
            | Key::Equals
    )
}

pub fn to_skey(k: Key) -> Option<SKey> {
    let s = match k {
        Key::T => SKey::T,
        Key::O => SKey::O,
        Key::E => SKey::E,
        Key::W => SKey::W,
        Key::R => SKey::R,
        Key::F => SKey::F,
        Key::C => SKey::C,
        Key::V => SKey::V,
        Key::Tab => SKey::Tab,
        Key::PageUp => SKey::PageUp,
        Key::PageDown => SKey::PageDown,
        Key::ArrowUp => SKey::ArrowUp,
        Key::ArrowDown => SKey::ArrowDown,
        Key::ArrowLeft => SKey::ArrowLeft,
        Key::ArrowRight => SKey::ArrowRight,
        _ => return None,
    };
    Some(s)
}

fn to_smods(m: &Modifiers) -> SMods {
    SMods {
        ctrl: m.ctrl || m.command,
        shift: m.shift,
        alt: m.alt,
    }
}

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

fn send_keypress(pane: PaneId, sess: &mut Session, key: GKey, mods: GMods) {
    let res = vtask::send_key(sess, |ev| {
        ev.set_action(GAction::Press);
        ev.set_key(key);
        ev.set_mods(mods);
        ev.set_utf8::<String>(None);
    });
    if let Err(e) = res {
        warn!("send key to pane {pane}: {e}");
    }
}

/// Frame-level input dispatch: shortcuts first, then terminal input.
pub fn handle(ctx: &Context, st: &mut AppState, sess: &mut SessionMap, ui: &mut UiState, dirty: &mut bool) {
    if ctx.egui_wants_keyboard_input() {
        return; // a text field has focus; let it keep the keys
    }
    let events = ctx.input(|i| i.events.clone());
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
                if let Some(p) = focused {
                    if let Some(s) = sess.map.get_mut(&p) {
                        for c in t.chars() {
                            send_char(p, s, c);
                        }
                    }
                }
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                if let Some(sk) = to_skey(key) {
                    if let Some(action) = state::route_shortcut(to_smods(&modifiers), sk) {
                        actions::apply_action(st, sess, ui, action, dirty);
                        continue;
                    }
                }
                let is_cmd = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
                if !is_cmd && produces_text(key) {
                    continue; // the matching Event::Text carries the character
                }
                let Some(gk) = ghostty_key(key) else { continue };
                let mods = ghostty_mods(&modifiers);
                if let Some(p) = focused {
                    if let Some(s) = sess.map.get_mut(&p) {
                        send_keypress(p, s, gk, mods);
                    }
                }
            }
            _ => {}
        }
    }
}
