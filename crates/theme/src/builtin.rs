//! Builtin theme registry.
//!
//! Five palettes stored as `const` RGB literals (canonical published values
//! from each theme's official palette); each constructor is a free function
//! returning an owned [`Palette`].

use crate::palette::{Palette, Rgb};

/// Registry keys of all builtin themes, in menu display order.
pub const BUILTIN_NAMES: &[&str] = &[
    "catppuccin-mocha",
    "tokyo-night",
    "dracula",
    "gruvbox-dark",
    "terminator-classic",
];

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

/// Catppuccin Mocha — https://catppuccin.com (ghostty/kitty ports).
pub fn catppuccin_mocha() -> Palette {
    Palette {
        name: "Catppuccin Mocha".into(),
        foreground: rgb(0xcd, 0xd6, 0xf4),   // text
        background: rgb(0x1e, 0x1e, 0x2e),   // base
        cursor: rgb(0xf5, 0xe0, 0xdc),       // rosewater
        selection_background: rgb(0x58, 0x5b, 0x70), // surface2
        block_highlight: rgb(0xcb, 0xa6, 0xf7), // mauve
        normal: [
            rgb(0x45, 0x47, 0x5a), // 0 surface1
            rgb(0xf3, 0x8b, 0xa8), // 1 red
            rgb(0xa6, 0xe3, 0xa1), // 2 green
            rgb(0xf9, 0xe2, 0xaf), // 3 yellow
            rgb(0x89, 0xb4, 0xfa), // 4 blue
            rgb(0xf5, 0xc2, 0xe7), // 5 pink
            rgb(0x94, 0xe2, 0xd5), // 6 teal
            rgb(0xba, 0xc2, 0xde), // 7 subtext1
        ],
        bright: [
            rgb(0x58, 0x5b, 0x70), // 8 surface2
            rgb(0xf3, 0x8b, 0xa8), // 9 red
            rgb(0xa6, 0xe3, 0xa1), // 10 green
            rgb(0xf9, 0xe2, 0xaf), // 11 yellow
            rgb(0x89, 0xb4, 0xfa), // 12 blue
            rgb(0xf5, 0xc2, 0xe7), // 13 pink
            rgb(0x94, 0xe2, 0xd5), // 14 teal
            rgb(0xa6, 0xad, 0xc8), // 15 subtext0
        ],
    }
}

/// Tokyo Night — folke/tokyo-night.nvim night variant.
pub fn tokyo_night() -> Palette {
    Palette {
        name: "Tokyo Night".into(),
        foreground: rgb(0xc0, 0xca, 0xf5),
        background: rgb(0x1a, 0x1b, 0x26),
        cursor: rgb(0xc0, 0xca, 0xf5),
        selection_background: rgb(0x33, 0x46, 0x7c),
        block_highlight: rgb(0x7a, 0xa2, 0xf7), // blue
        normal: [
            rgb(0x15, 0x16, 0x1e), // 0 black
            rgb(0xf7, 0x76, 0x8e), // 1 red
            rgb(0x9e, 0xce, 0x6a), // 2 green
            rgb(0xe0, 0xaf, 0x68), // 3 yellow
            rgb(0x7a, 0xa2, 0xf7), // 4 blue
            rgb(0xbb, 0x9a, 0xf7), // 5 magenta
            rgb(0x7d, 0xcf, 0xff), // 6 cyan
            rgb(0xa9, 0xb1, 0xd6), // 7 fg_dark
        ],
        bright: [
            rgb(0x41, 0x48, 0x68), // 8 terminal_black
            rgb(0xff, 0x7a, 0x93), // 9 red
            rgb(0xb9, 0xf2, 0x7c), // 10 green
            rgb(0xff, 0x9e, 0x64), // 11 orange
            rgb(0x7d, 0xa6, 0xff), // 12 blue
            rgb(0xbb, 0x9a, 0xf7), // 13 magenta
            rgb(0x0d, 0xb9, 0xd7), // 14 cyan
            rgb(0xc0, 0xca, 0xf5), // 15 fg
        ],
    }
}

/// Dracula — https://dracula.github.io (iTerm2/ghostty ports).
pub fn dracula() -> Palette {
    Palette {
        name: "Dracula".into(),
        foreground: rgb(0xf8, 0xf8, 0xf2),
        background: rgb(0x28, 0x2a, 0x36),
        cursor: rgb(0xf8, 0xf8, 0xf2),
        selection_background: rgb(0x44, 0x47, 0x5a), // current line
        block_highlight: rgb(0xbd, 0x93, 0xf9),      // purple
        normal: [
            rgb(0x21, 0x22, 0x2c), // 0 black
            rgb(0xff, 0x55, 0x55), // 1 red
            rgb(0x50, 0xfa, 0x7b), // 2 green
            rgb(0xf1, 0xfa, 0x8c), // 3 yellow
            rgb(0xbd, 0x93, 0xf9), // 4 purple -> blue slot
            rgb(0xff, 0x79, 0xc6), // 5 pink
            rgb(0x8b, 0xe9, 0xfd), // 6 cyan
            rgb(0xf8, 0xf8, 0xf2), // 7 white
        ],
        bright: [
            rgb(0x68, 0x68, 0x70), // 8 bright black
            rgb(0xff, 0x6e, 0x67), // 9 bright red
            rgb(0x5a, 0xf7, 0x8e), // 10 bright green
            rgb(0xf4, 0xf9, 0x9d), // 11 bright yellow
            rgb(0xca, 0xa9, 0xfa), // 12 bright purple
            rgb(0xff, 0x92, 0xd0), // 13 bright pink
            rgb(0x9a, 0xed, 0xfe), // 14 bright cyan
            rgb(0xf8, 0xf8, 0xf2), // 15 bright white
        ],
    }
}

/// Gruvbox Dark — https://github.com/morhetz/gruvbox (medium variant).
pub fn gruvbox_dark() -> Palette {
    Palette {
        name: "Gruvbox Dark".into(),
        foreground: rgb(0xeb, 0xdb, 0xb2), // fg2 / bright white
        background: rgb(0x28, 0x28, 0x28), // bg0 medium
        cursor: rgb(0xeb, 0xdb, 0xb2),
        selection_background: rgb(0x32, 0x30, 0x2f), // bg1
        block_highlight: rgb(0xfe, 0x80, 0x19),      // bright orange
        normal: [
            rgb(0x28, 0x28, 0x28), // 0 bg0
            rgb(0xcc, 0x24, 0x1d), // 1 neutral red
            rgb(0x98, 0x97, 0x1a), // 2 neutral green
            rgb(0xd7, 0x99, 0x21), // 3 neutral yellow
            rgb(0x45, 0x85, 0x88), // 4 neutral blue
            rgb(0xb1, 0x62, 0x86), // 5 neutral purple
            rgb(0x68, 0x9d, 0x6a), // 6 neutral aqua
            rgb(0xa8, 0x99, 0x84), // 7 fg4 / gray
        ],
        bright: [
            rgb(0x92, 0x83, 0x74), // 8 neutral gray
            rgb(0xfb, 0x49, 0x34), // 9 bright red
            rgb(0xb8, 0xbb, 0x26), // 10 bright green
            rgb(0xfa, 0xbd, 0x2f), // 11 bright yellow
            rgb(0x83, 0xa5, 0x98), // 12 bright blue
            rgb(0xd3, 0x86, 0x9b), // 13 bright purple
            rgb(0x8e, 0xc0, 0x7c), // 14 bright aqua
            rgb(0xeb, 0xdb, 0xb2), // 15 fg2
        ],
    }
}

/// Terminator Classic — gnome-terminator's signature purple over the Tango
/// 16-color set that shipped as its default.
pub fn terminator_classic() -> Palette {
    Palette {
        name: "Terminator Classic".into(),
        foreground: rgb(0xff, 0xff, 0xff),
        background: rgb(0x30, 0x0a, 0x24), // gnome-terminator purple
        cursor: rgb(0xff, 0xff, 0xff),
        selection_background: rgb(0x75, 0x50, 0x7b), // Tango plum
        block_highlight: rgb(0xad, 0x7f, 0xa8),     // Tango bright plum
        normal: [
            rgb(0x00, 0x00, 0x00), // 0 black
            rgb(0xcc, 0x00, 0x00), // 1 scarlet red
            rgb(0x4e, 0x9a, 0x06), // 2 chameleon
            rgb(0xc4, 0xa0, 0x00), // 3 banana (dark)
            rgb(0x34, 0x65, 0xa4), // 4 sky blue (dark)
            rgb(0x75, 0x50, 0x7b), // 5 plum (dark)
            rgb(0x06, 0x98, 0x9a), // 6 cyan (dark)
            rgb(0xd3, 0xd7, 0xcf), // 7 aluminium
        ],
        bright: [
            rgb(0x55, 0x57, 0x53), // 8 aluminium dark
            rgb(0xef, 0x29, 0x29), // 9 scarlet red
            rgb(0x8a, 0xe2, 0x34), // 10 chameleon
            rgb(0xfc, 0xe9, 0x4f), // 11 banana
            rgb(0x72, 0x9f, 0xcf), // 12 sky blue
            rgb(0xad, 0x7f, 0xa8), // 13 plum
            rgb(0x34, 0xe2, 0xe2), // 14 cyan
            rgb(0xee, 0xee, 0xec), // 15 aluminium light
        ],
    }
}

/// Registry keys of all builtin themes.
pub fn builtin_names() -> Vec<&'static str> {
    BUILTIN_NAMES.to_vec()
}

/// Look up a builtin palette by registry key, case-insensitively.
pub fn builtin_by_name(name: &str) -> Option<Palette> {
    match name.to_ascii_lowercase().as_str() {
        "catppuccin-mocha" => Some(catppuccin_mocha()),
        "tokyo-night" => Some(tokyo_night()),
        "dracula" => Some(dracula()),
        "gruvbox-dark" => Some(gruvbox_dark()),
        "terminator-classic" => Some(terminator_classic()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constructors() -> Vec<(&'static str, Palette)> {
        BUILTIN_NAMES
            .iter()
            .map(|n| (*n, builtin_by_name(n).expect("builtin present")))
            .collect()
    }

    #[test]
    fn registry_has_five_entries() {
        assert_eq!(BUILTIN_NAMES.len(), 5);
        assert_eq!(builtin_names().len(), 5);
    }

    fn mixed_case(s: &str) -> String {
        s.char_indices()
            .map(|(i, c)| if i % 2 == 0 { c.to_ascii_uppercase() } else { c })
            .collect()
    }

    #[test]
    fn builtin_by_name_is_case_insensitive() {
        for name in BUILTIN_NAMES {
            assert!(builtin_by_name(name).is_some(), "missing {name}");
            assert!(builtin_by_name(&name.to_uppercase()).is_some());
            assert!(builtin_by_name(&mixed_case(name)).is_some());
            let p = builtin_by_name(&name.to_uppercase()).expect("present");
            assert_eq!(p.name.to_ascii_lowercase().replace(' ', "-"), *name);
        }
        assert!(builtin_by_name("no-such-theme").is_none());
        assert!(builtin_by_name("").is_none());
    }

    #[test]
    fn every_builtin_is_visible_on_its_background() {
        for (key, p) in constructors() {
            assert_ne!(p.background, p.foreground, "{key} fg/bg identical");
            assert_ne!(p.background, p.cursor, "{key} cursor invisible");
            assert_ne!(p.background, p.block_highlight, "{key} highlight invisible");
        }
    }

    #[test]
    fn all_builtins_have_distinct_backgrounds() {
        let all = constructors();
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i].1.background, all[j].1.background);
            }
        }
    }

    #[test]
    fn palette_roundtrips_through_serde_json() {
        let p = catppuccin_mocha();
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Palette = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, back);
        assert!(json.contains("\"name\":\"Catppuccin Mocha\""));
    }

    #[test]
    fn builtin_hex_values_are_canonical() {
        assert_eq!(
            crate::palette::rgb_to_hex(gruvbox_dark().background),
            "#282828"
        );
        assert_eq!(
            crate::palette::rgb_to_hex(dracula().background),
            "#282a36"
        );
        assert_eq!(
            crate::palette::rgb_to_hex(tokyo_night().foreground),
            "#c0caf5"
        );
        assert_eq!(
            crate::palette::rgb_to_hex(terminator_classic().background),
            "#300a24"
        );
        assert_eq!(
            crate::palette::rgb_to_hex(catppuccin_mocha().background),
            "#1e1e2e"
        );
    }
}
