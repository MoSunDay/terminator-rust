//! Pure egui <-> ghostty / shortcut key mappings (no side effects).

use egui::{Event, Key, Modifiers};
use libghostty_vt::key::{Key as GKey, Mods as GMods};

use crate::state::{SKey, SMods};

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

pub fn to_smods(m: &Modifiers) -> SMods {
    SMods {
        ctrl: m.ctrl || m.command,
        shift: m.shift,
        alt: m.alt,
    }
}


/// Printable char for a key, shift applied.
pub fn key_char(key: Key, shift: bool) -> Option<char> {
    let (unshifted, shifted) = match key {
        Key::A => ('a', 'A'),
        Key::B => ('b', 'B'),
        Key::C => ('c', 'C'),
        Key::D => ('d', 'D'),
        Key::E => ('e', 'E'),
        Key::F => ('f', 'F'),
        Key::G => ('g', 'G'),
        Key::H => ('h', 'H'),
        Key::I => ('i', 'I'),
        Key::J => ('j', 'J'),
        Key::K => ('k', 'K'),
        Key::L => ('l', 'L'),
        Key::M => ('m', 'M'),
        Key::N => ('n', 'N'),
        Key::O => ('o', 'O'),
        Key::P => ('p', 'P'),
        Key::Q => ('q', 'Q'),
        Key::R => ('r', 'R'),
        Key::S => ('s', 'S'),
        Key::T => ('t', 'T'),
        Key::U => ('u', 'U'),
        Key::V => ('v', 'V'),
        Key::W => ('w', 'W'),
        Key::X => ('x', 'X'),
        Key::Y => ('y', 'Y'),
        Key::Z => ('z', 'Z'),
        Key::Num0 => ('0', '0'),
        Key::Num1 => ('1', '1'),
        Key::Num2 => ('2', '2'),
        Key::Num3 => ('3', '3'),
        Key::Num4 => ('4', '4'),
        Key::Num5 => ('5', '5'),
        Key::Num6 => ('6', '6'),
        Key::Num7 => ('7', '7'),
        Key::Num8 => ('8', '8'),
        Key::Num9 => ('9', '9'),
        Key::Space => (' ', ' '),
        Key::Comma => (',', '<'),
        Key::Minus => ('-', '_'),
        Key::Period => ('.', '>'),
        Key::Slash => ('/', '?'),
        Key::Semicolon => (';', ':'),
        Key::Quote => ('\'', '"'),
        Key::Backslash => ('\\', '|'),
        Key::Backtick => ('`', '~'),
        Key::OpenBracket => ('[', '{'),
        Key::CloseBracket => (']', '}'),
        Key::Equals => ('=', '+'),
        _ => return None,
    };
    Some(if shift { shifted } else { unshifted })
}

/// Lowercased chars of this frame's Alt-modified Key presses. egui-winit
/// emits Key + Text together on X11, so those Text events are duplicates.
pub fn alt_keyed_chars(events: &[Event]) -> Vec<char> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if modifiers.alt => {
                key_char(*key, modifiers.shift).map(|c| c.to_ascii_lowercase())
            }
            _ => None,
        })
        .collect()
}

/// True when a Key event must bypass the Text path and be encoded with
/// its modifiers: ctrl/cmd combos and Alt meta keys (bash Alt+b/f/d ...).
pub fn is_command_key(m: &Modifiers) -> bool {
    m.ctrl || m.command || m.mac_cmd || m.alt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool, alt: bool) -> Modifiers {
        Modifiers {
            ctrl,
            shift,
            alt,
            mac_cmd: false,
            command: false,
        }
    }

    #[test]
    fn alt_and_ctrl_are_command_keys() {
        // Alt+letter must reach the encoder as ESC-prefixed meta keys.
        assert!(is_command_key(&mods(false, false, true)));
        assert!(is_command_key(&mods(true, false, false)));
        assert!(!is_command_key(&mods(false, false, false)));
        assert!(!is_command_key(&mods(false, true, false)));
    }

    #[test]
    fn ghostty_mods_maps_all_four() {
        let m = Modifiers {
            mac_cmd: true,
            ..mods(true, true, true)
        };
        let g = ghostty_mods(&m);
        assert!(g.contains(GMods::CTRL));
        assert!(g.contains(GMods::SHIFT));
        assert!(g.contains(GMods::ALT));
        assert!(g.contains(GMods::SUPER));
    }

    #[test]
    fn key_char_shifts_and_filters() {
        assert_eq!(key_char(Key::B, false), Some('b'));
        assert_eq!(key_char(Key::B, true), Some('B'));
        assert_eq!(key_char(Key::Num3, true), Some('3'));
        assert_eq!(key_char(Key::Escape, true), None);
    }

    #[test]
    fn key_char_prints_punctuation() {
        assert_eq!(key_char(Key::Space, false), Some(' '));
        assert_eq!(key_char(Key::Space, true), Some(' '));
        assert_eq!(key_char(Key::Comma, false), Some(','));
        assert_eq!(key_char(Key::Comma, true), Some('<'));
        assert_eq!(key_char(Key::Slash, true), Some('?'));
        assert_eq!(key_char(Key::Backtick, false), Some('`'));
        assert_eq!(key_char(Key::Backtick, true), Some('~'));
        assert_eq!(key_char(Key::Quote, false), Some('\''));
        assert_eq!(key_char(Key::Quote, true), Some('"'));
    }

    #[test]
    fn alt_keyed_chars_collects_lowercase() {
        let events = vec![Event::Key {
            key: Key::B,
            pressed: true,
            repeat: false,
            physical_key: None,
            modifiers: Modifiers::ALT,
        }];
        assert_eq!(alt_keyed_chars(&events), vec!['b']);
    }

    #[test]
    fn alt_keyed_chars_collects_punctuation() {
        let events = vec![Event::Key {
            key: Key::Comma,
            pressed: true,
            repeat: false,
            physical_key: None,
            modifiers: Modifiers::ALT,
        }];
        // to_ascii_lowercase is a no-op for punctuation.
        assert_eq!(alt_keyed_chars(&events), vec![',']);
    }
}
