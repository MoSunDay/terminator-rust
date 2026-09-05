//! Core color types, parsing, and indexed-color mapping.
//!
//! Everything here is a plain data struct or a free function: no traits, no
//! dynamic dispatch, no renderer types.

use serde::{Deserialize, Serialize};

/// 8-bit-per-channel RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Error produced by [`parse_hex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseHexError {
    /// The string (after an optional leading `#`) is not 3 or 6 characters.
    InvalidLength,
    /// At least one character is not a hexadecimal digit.
    InvalidDigit,
}

impl std::fmt::Display for ParseHexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseHexError::InvalidLength => {
                write!(f, "invalid hex color length (expected 3 or 6 digits)")
            }
            ParseHexError::InvalidDigit => {
                write!(f, "invalid hex color digit")
            }
        }
    }
}

impl std::error::Error for ParseHexError {}

/// A 16-color ANSI palette plus UI chrome colors (background, cursor,
/// selection, pane highlight). Design borrowed from otty's ColorPalette:
/// 16 base colors + bright variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    pub name: String,
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub selection_background: Rgb,
    /// Highlight color for split dividers / focused pane ring.
    pub block_highlight: Rgb,
    /// black red green yellow blue magenta cyan white
    pub normal: [Rgb; 8],
    /// bright variants, same slot order as [`Palette::normal`]
    pub bright: [Rgb; 8],
}

/// Standard xterm 6x6x6 cube component ramp for indexed colors 16..=231.
const XTERM_CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn hex_pair(hi: u8, lo: u8) -> Result<u8, ParseHexError> {
    match (hex_digit(hi), hex_digit(lo)) {
        (Some(h), Some(l)) => Ok(h * 16 + l),
        _ => Err(ParseHexError::InvalidDigit),
    }
}

/// Parse a hex color: `#rrggbb`, `rrggbb`, `#rgb` or `rgb` (short form
/// expands each digit, e.g. `#abc` = `#aabbcc`). Case-insensitive.
pub fn parse_hex(s: &str) -> Result<Rgb, ParseHexError> {
    let body = s.strip_prefix('#').unwrap_or(s);
    let d = body.as_bytes();
    match d.len() {
        6 => Ok(Rgb {
            r: hex_pair(d[0], d[1])?,
            g: hex_pair(d[2], d[3])?,
            b: hex_pair(d[4], d[5])?,
        }),
        3 => Ok(Rgb {
            r: hex_pair(d[0], d[0])?,
            g: hex_pair(d[1], d[1])?,
            b: hex_pair(d[2], d[2])?,
        }),
        _ => Err(ParseHexError::InvalidLength),
    }
}

/// Format a color as lowercase `#rrggbb`.
pub fn rgb_to_hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// Linear interpolation of one channel: `a * (1 - t) + b * t`, rounded and
/// clamped into `0..=255`. Callers must pass a `t` already in `0.0..=1.0`.
fn lerp_channel(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 * (1.0 - t) + b as f32 * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Linear blend from `a` to `b` at factor `t` (clamped to `0.0..=1.0`):
/// `t = 0` yields `a`, `t = 1` yields `b`.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    Rgb {
        r: lerp_channel(a.r, b.r, t),
        g: lerp_channel(a.g, b.g, t),
        b: lerp_channel(a.b, b.b, t),
    }
}

/// Composite `fg` over `bg` with `alpha` clamped to `0.0..=1.0`
/// (`0.0` = fully `bg`, `1.0` = fully `fg`). Used for pane transparency:
/// a pane background color blended over the theme background.
pub fn with_alpha_over(fg: Rgb, bg: Rgb, alpha: f32) -> Rgb {
    let a = alpha.clamp(0.0, 1.0);
    Rgb {
        r: lerp_channel(bg.r, fg.r, a),
        g: lerp_channel(bg.g, fg.g, a),
        b: lerp_channel(bg.b, fg.b, a),
    }
}

/// Map a terminal indexed color (0-255) to a concrete RGB:
/// - `0..=7`: [`Palette::normal`] slots
/// - `8..=15`: [`Palette::bright`] slots
/// - `16..=231`: xterm 6x6x6 cube with the standard ramp
///   `[0, 95, 135, 175, 215, 255]`
/// - `232..=255`: grayscale ramp `8 + 10 * (idx - 232)`
pub fn indexed_color(p: &Palette, idx: u8) -> Rgb {
    match idx {
        0..=7 => p.normal[idx as usize],
        8..=15 => p.bright[(idx - 8) as usize],
        16..=231 => {
            let i = idx - 16;
            let r = XTERM_CUBE[(i / 36) as usize];
            let g = XTERM_CUBE[((i / 6) % 6) as usize];
            let b = XTERM_CUBE[(i % 6) as usize];
            Rgb { r, g, b }
        }
        _ => {
            let v = 8 + 10 * (idx - 232);
            Rgb { r: v, g: v, b: v }
        }
    }
}

/// Effective pane background for visual (cross-platform, no WM compositing)
/// transparency: `effective_bg = with_alpha_over(pane_color, background, 1 - t)`.
///
/// `pane_color` is the user's chosen pane tint; `transparency` `0.0` = fully
/// opaque pane color, `1.0` = fully shows the theme background. When no pane
/// color is set the palette background is used, so the result is always the
/// theme background regardless of transparency.
pub fn blend_background(p: &Palette, pane_color: Option<Rgb>, transparency: f32) -> Rgb {
    let pane = pane_color.unwrap_or(p.background);
    let alpha = 1.0 - transparency.clamp(0.0, 1.0);
    with_alpha_over(pane, p.background, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn test_palette() -> Palette {
        Palette {
            name: "test".into(),
            foreground: rgb(255, 255, 255),
            background: rgb(0, 0, 0),
            cursor: rgb(255, 255, 255),
            selection_background: rgb(51, 51, 51),
            block_highlight: rgb(0, 128, 255),
            normal: [
                rgb(0, 0, 0),
                rgb(205, 0, 0),
                rgb(0, 205, 0),
                rgb(205, 205, 0),
                rgb(0, 0, 238),
                rgb(205, 0, 205),
                rgb(0, 205, 205),
                rgb(229, 229, 229),
            ],
            bright: [
                rgb(127, 127, 127),
                rgb(255, 0, 0),
                rgb(0, 255, 0),
                rgb(255, 255, 0),
                rgb(92, 92, 255),
                rgb(255, 0, 255),
                rgb(0, 255, 255),
                rgb(255, 255, 255),
            ],
        }
    }

    #[test]
    fn parse_hex_valid() {
        assert_eq!(parse_hex("#ff0000"), Ok(rgb(255, 0, 0)));
        assert_eq!(parse_hex("00ff00"), Ok(rgb(0, 255, 0)));
        assert_eq!(parse_hex("#abc"), Ok(rgb(170, 187, 204)));
        assert_eq!(parse_hex("ABC"), Ok(rgb(170, 187, 204)));
        assert_eq!(parse_hex("#1e1e2e"), Ok(rgb(30, 30, 46)));
    }

    #[test]
    fn parse_hex_invalid() {
        assert_eq!(parse_hex("xyz"), Err(ParseHexError::InvalidDigit));
        assert_eq!(parse_hex(""), Err(ParseHexError::InvalidLength));
        assert_eq!(parse_hex("#1234567"), Err(ParseHexError::InvalidLength));
        assert_eq!(parse_hex("#12345"), Err(ParseHexError::InvalidLength));
        assert_eq!(parse_hex("#12345x"), Err(ParseHexError::InvalidDigit));
    }

    #[test]
    fn hex_roundtrip() {
        for s in ["#ff0000", "#00ff00", "#0000ff", "#282828", "#ebdbb2"] {
            let c = parse_hex(s).expect("valid hex");
            assert_eq!(rgb_to_hex(c), s);
            assert_eq!(parse_hex(&rgb_to_hex(c)), Ok(c));
        }
        assert_eq!(rgb_to_hex(rgb(0, 0, 0)), "#000000");
    }

    #[test]
    fn parse_hex_error_display() {
        let e = parse_hex("xyz").expect_err("invalid digit");
        assert!(e.to_string().contains("digit"));
        let e = parse_hex("#1").expect_err("too short");
        assert!(e.to_string().contains("length"));
        assert_eq!(parse_hex("zz"), Err(ParseHexError::InvalidLength));
        let _: &dyn std::error::Error = &e;
    }

    #[test]
    fn indexed_color_maps_palette_slots() {
        let p = test_palette();
        assert_eq!(indexed_color(&p, 0), p.normal[0]);
        assert_eq!(indexed_color(&p, 7), p.normal[7]);
        assert_eq!(indexed_color(&p, 8), p.bright[0]);
        assert_eq!(indexed_color(&p, 15), p.bright[7]);
    }

    #[test]
    fn indexed_color_xterm_cube() {
        let p = test_palette();
        assert_eq!(indexed_color(&p, 16), rgb(0, 0, 0));
        // idx 16 + 5 = pure blue corner of the cube.
        assert_eq!(indexed_color(&p, 21), rgb(0, 0, 255));
        // idx 16 + 36 = one red step up.
        assert_eq!(indexed_color(&p, 52), rgb(95, 0, 0));
        // 5*36 + 5*6 + 5 = white corner.
        assert_eq!(indexed_color(&p, 231), rgb(255, 255, 255));
    }

    #[test]
    fn indexed_color_grayscale() {
        let p = test_palette();
        assert_eq!(indexed_color(&p, 232), rgb(8, 8, 8));
        assert_eq!(indexed_color(&p, 244), rgb(128, 128, 128));
        assert_eq!(indexed_color(&p, 255), rgb(238, 238, 238));
    }

    #[test]
    fn mix_clamps_t() {
        let a = rgb(0, 0, 0);
        let b = rgb(255, 255, 255);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, -3.0), a);
        assert_eq!(mix(a, b, 4.0), b);
        assert_eq!(mix(a, b, 0.5), rgb(128, 128, 128));
    }

    #[test]
    fn with_alpha_over_edges() {
        let fg = rgb(255, 0, 0);
        let bg = rgb(0, 0, 255);
        assert_eq!(with_alpha_over(fg, bg, 1.0), fg);
        assert_eq!(with_alpha_over(fg, bg, 0.0), bg);
        assert_eq!(with_alpha_over(fg, bg, 2.0), fg);
        assert_eq!(with_alpha_over(fg, bg, -1.0), bg);
        assert_eq!(with_alpha_over(fg, bg, 0.5), rgb(128, 0, 128));
        assert_eq!(with_alpha_over(fg, fg, 0.5), fg);
    }

    #[test]
    fn blend_background_with_pane_color() {
        let p = test_palette();
        let tint = rgb(200, 100, 50);
        // t = 0: opaque user pane color.
        assert_eq!(blend_background(&p, Some(tint), 0.0), tint);
        // t = 1: fully transparent, shows the theme background.
        assert_eq!(blend_background(&p, Some(tint), 1.0), p.background);
        // out-of-range transparency is clamped.
        assert_eq!(blend_background(&p, Some(tint), -5.0), tint);
        assert_eq!(blend_background(&p, Some(tint), 5.0), p.background);
        // halfway: tint over black background at alpha 0.5.
        assert_eq!(
            blend_background(&p, Some(tint), 0.5),
            with_alpha_over(tint, p.background, 0.5)
        );
    }

    #[test]
    fn blend_background_without_pane_color() {
        let p = test_palette();
        assert_eq!(blend_background(&p, None, 0.0), p.background);
        assert_eq!(blend_background(&p, None, 1.0), p.background);
        assert_eq!(blend_background(&p, None, 0.37), p.background);
    }
}
