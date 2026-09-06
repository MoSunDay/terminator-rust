//! Terminal color themes: indexed-color (0-255) palettes with hex parsing,
//! blending helpers, and builtin themes.
//!
//! Render-agnostic: exposes plain `Rgb` values that UI layers convert to
//! their own color types at draw time.

pub mod builtin;
pub mod palette;

pub use builtin::{BUILTIN_NAMES, builtin_by_name, builtin_names};
pub use palette::{
    Palette, ParseHexError, Rgb, blend_background, indexed_color, is_dark, mix, parse_hex,
    rgb_to_hex, with_alpha_over,
};
