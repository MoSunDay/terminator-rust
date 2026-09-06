//! Theme + per-pane color resolution to egui Color32.

use egui::Color32;
use theme::{
    blend_background, builtin_by_name, is_dark as theme_is_dark, rgb_to_hex, Palette, Rgb,
    BUILTIN_NAMES,
};
use vt_pane::term::Color as VtColor;
use vt_pane::CellData;

use crate::state::PaneMeta;

pub fn to_c32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// egui Color32 -> theme Rgb (for re-blending with theme helpers).
pub fn from_c32(c: Color32) -> Rgb {
    Rgb {
        r: c.r(),
        g: c.g(),
        b: c.b(),
    }
}

pub fn vt_rgb(c: VtColor) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

fn gray(v: u8) -> Rgb {
    Rgb { r: v, g: v, b: v }
}

/// Last-resort palette when no builtin resolves (never happens in practice).
pub fn fallback_palette() -> Palette {
    let normal = [
        gray(0),
        Rgb {
            r: 205,
            g: 49,
            b: 49,
        },
        Rgb {
            r: 13,
            g: 188,
            b: 121,
        },
        Rgb {
            r: 229,
            g: 229,
            b: 16,
        },
        Rgb {
            r: 36,
            g: 114,
            b: 200,
        },
        Rgb {
            r: 188,
            g: 63,
            b: 188,
        },
        Rgb {
            r: 17,
            g: 168,
            b: 205,
        },
        gray(192),
    ];
    let bright = [
        gray(64),
        Rgb {
            r: 241,
            g: 76,
            b: 76,
        },
        Rgb {
            r: 23,
            g: 212,
            b: 110,
        },
        Rgb {
            r: 245,
            g: 245,
            b: 66,
        },
        Rgb {
            r: 80,
            g: 150,
            b: 240,
        },
        Rgb {
            r: 220,
            g: 100,
            b: 220,
        },
        Rgb {
            r: 80,
            g: 220,
            b: 240,
        },
        gray(230),
    ];
    Palette {
        name: "fallback".to_string(),
        foreground: gray(192),
        background: gray(30),
        cursor: gray(220),
        selection_background: gray(60),
        block_highlight: Rgb {
            r: 80,
            g: 80,
            b: 120,
        },
        normal,
        bright,
    }
}

/// Resolve the active palette by theme name (falls back to the first builtin).
pub fn palette_of(name: &str) -> Palette {
    builtin_by_name(name)
        .or_else(|| BUILTIN_NAMES.first().and_then(|n| builtin_by_name(n)))
        .unwrap_or_else(fallback_palette)
}

/// True when the named theme's background is dark (feeds OSC color-scheme answers).
pub fn is_dark(name: &str) -> bool {
    theme_is_dark(&palette_of(name))
}

/// Nine hex slots for remote bootstrap: fg, bg, red..cyan, orange.
pub fn palette_hex(p: &Palette) -> [String; 9] {
    [
        rgb_to_hex(p.foreground),
        rgb_to_hex(p.background),
        rgb_to_hex(p.normal[0]),
        rgb_to_hex(p.normal[1]),
        rgb_to_hex(p.normal[2]),
        rgb_to_hex(p.normal[3]),
        rgb_to_hex(p.normal[4]),
        rgb_to_hex(p.normal[5]),
        rgb_to_hex(p.bright[0]),
    ]
}

/// Effective pane background: pane color over theme, blended by transparency.
pub fn effective_bg(p: &Palette, meta: &PaneMeta) -> Color32 {
    to_c32(blend_background(
        p,
        meta.bg_color,
        meta.transparency.clamp(0.0, 1.0),
    ))
}

/// Linear channel mix: `a` toward `b` by `t` (0 -> a, 1 -> b), rounded.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + t * (f32::from(y) - f32::from(x))).round() as u8;
    Rgb {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
    }
}

/// Chrome surface: panel/window fill, one step above the theme bg.
pub fn chrome_bg(p: &Palette) -> Rgb {
    mix(p.background, p.foreground, 0.07)
}
/// Hovered chrome (chips, icon buttons).
pub fn chrome_hover(p: &Palette) -> Rgb {
    mix(p.background, p.foreground, 0.14)
}
/// Dividers, hairlines, scrollbar tracks.
pub fn divider(p: &Palette) -> Rgb {
    mix(p.background, p.foreground, 0.16)
}
/// Active tab chip fill.
pub fn tab_active(p: &Palette) -> Rgb {
    mix(p.background, p.block_highlight, 0.35)
}
/// Dimmed chrome text (window title).
pub fn title_text(p: &Palette) -> Rgb {
    mix(p.foreground, p.background, 0.30)
}

/// Swatch candidates for the pane color popup: palette 16 colors + basics.
pub fn swatches(p: &Palette) -> Vec<Rgb> {
    let mut out: Vec<Rgb> = p.normal.to_vec();
    out.extend(p.bright);
    out.push(Rgb { r: 0, g: 0, b: 0 });
    out.push(gray(128));
    out.push(gray(255));
    out
}

/// Resolve a cell's (fg, bg): explicit overrides win, inverse swaps them.
/// `bg == None` means "paint the pane default background".
pub fn cell_colors(
    cell: &CellData,
    default_fg: Color32,
    default_bg: Color32,
) -> (Color32, Option<Color32>) {
    let fg = cell.fg.map(vt_rgb).map_or(default_fg, to_c32);
    let bg = cell.bg.map(vt_rgb).map_or(default_bg, to_c32);
    if cell.inverse {
        (bg, Some(fg))
    } else if cell.bg.is_some() {
        (fg, Some(bg))
    } else {
        (fg, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_of_falls_back() {
        assert_eq!(palette_of("dracula").name.to_lowercase(), "dracula");
        assert_ne!(palette_of("").name, "");
        assert_eq!(
            palette_of("no-such-theme").name,
            palette_of(BUILTIN_NAMES[0]).name
        );
    }

    #[test]
    fn palette_hex_has_nine_slots() {
        let p = palette_of("tokyo-night");
        let hex = palette_hex(&p);
        assert_eq!(hex.len(), 9);
        for h in &hex {
            assert!(h.starts_with('#') && h.len() == 7, "bad hex {h}");
        }
        assert_eq!(hex[0], rgb_to_hex(p.foreground));
        assert_eq!(hex[2], rgb_to_hex(p.normal[0]));
    }

    #[test]
    fn chrome_mix_derives_expected_values() {
        let p = palette_of("dracula");
        // Hand-computed: mix((40,42,54), (248,248,242), 0.16) -> (73,75,84).
        assert_eq!(
            divider(&p),
            Rgb {
                r: 73,
                g: 75,
                b: 84
            }
        );
        let bg = chrome_bg(&p);
        let hover = chrome_hover(&p);
        let div = divider(&p);
        assert_ne!(bg, hover);
        assert_ne!(hover, div);
        assert_ne!(bg, div);
        assert!(
            bg.r < hover.r && bg.r < div.r,
            "chrome_bg must be the darkest chrome step"
        );
        let a = Rgb {
            r: 10,
            g: 20,
            b: 30,
        };
        let b = Rgb {
            r: 200,
            g: 100,
            b: 0,
        };
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, -1.0), a);
    }

    #[test]
    fn cell_colors_resolve_explicit_and_inverse() {
        let fg = Color32::from_rgb(1, 2, 3);
        let bg = Color32::from_rgb(4, 5, 6);
        let mut c = CellData::default();
        assert_eq!(cell_colors(&c, fg, bg), (fg, None));
        c.fg = Some(VtColor { r: 9, g: 9, b: 9 });
        assert_eq!(cell_colors(&c, fg, bg).0, Color32::from_rgb(9, 9, 9));
        c.bg = Some(VtColor { r: 8, g: 8, b: 8 });
        assert_eq!(cell_colors(&c, fg, bg).1, Some(Color32::from_rgb(8, 8, 8)));
        c.inverse = true;
        let (ifg, ibg) = cell_colors(&c, fg, bg);
        assert_eq!(ifg, Color32::from_rgb(8, 8, 8));
        assert_eq!(ibg, Some(Color32::from_rgb(9, 9, 9)));
    }
}
