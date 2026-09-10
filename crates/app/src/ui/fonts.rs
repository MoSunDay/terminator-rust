//! Embedded CJK fallback font.
//!
//! Ships a Noto Sans SC subset (OFL, see `assets/fonts/OFL.txt`) and
//! registers it as the LAST entry of the monospace and proportional
//! family chains, so Han/kana/fullwidth glyphs fall back to it in any
//! text drawn by egui. `TERMINATOR_CJK_FONT=path[:face_index]` swaps in
//! a system font file (e.g. a multi-face `.ttc`) instead.

use std::path::PathBuf;

use egui::{FontData, FontDefinitions, FontFamily};

/// Registered name of the CJK fallback font.
pub const CJK_FONT_NAME: &str = "terminator-cjk";

/// Embedded Noto Sans SC subset (OFL).
const CJK_FONT_BYTES: &[u8] =
    include_bytes!("../../../../assets/fonts/NotoSansSC-Regular-subset.otf");

/// Parse a `path[:face_index]` font spec into (path, TTC face index).
///
/// The suffix after the LAST ':' counts as an index only when it parses
/// as `u32`; anything else (e.g. a Windows drive colon) means the whole
/// string is the path and the index is 0.
pub fn parse_font_spec(spec: &str) -> (PathBuf, u32) {
    let spec = spec.trim();
    match spec.rsplit_once(':') {
        Some((path, idx)) => match idx.trim().parse::<u32>() {
            Ok(index) => (PathBuf::from(path.trim()), index),
            Err(_) => (PathBuf::from(spec), 0),
        },
        None => (PathBuf::from(spec), 0),
    }
}

/// Font data from `TERMINATOR_CJK_FONT` when the variable is set and the
/// file is readable; `None` falls back to the embedded bytes.
pub fn env_font_data() -> Option<FontData> {
    let spec = std::env::var("TERMINATOR_CJK_FONT").ok()?;
    if spec.trim().is_empty() {
        return None;
    }
    let (path, index) = parse_font_spec(&spec);
    match std::fs::read(&path) {
        Ok(bytes) => {
            log::info!("CJK font from env: {} face {index}", path.display());
            Some(FontData {
                index,
                ..FontData::from_owned(bytes)
            })
        }
        Err(e) => {
            log::warn!(
                "cannot read TERMINATOR_CJK_FONT {}: {e}; using embedded font",
                path.display()
            );
            None
        }
    }
}

/// Append the CJK font to the end of both family fallback chains.
/// Idempotent: a second call must not duplicate the entry.
fn push_families(defs: &mut FontDefinitions) {
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        let list = defs.families.entry(family).or_default();
        if !list.iter().any(|name| name == CJK_FONT_NAME) {
            list.push(CJK_FONT_NAME.to_string());
        }
    }
}

/// Install the CJK fallback font on the context. Always succeeds (the
/// embedded bytes are compiled in); returns true for symmetry with a
/// future richer policy.
pub fn install(ctx: &egui::Context) -> bool {
    let mut defs = FontDefinitions::default();
    let data = match env_font_data() {
        Some(d) => d,
        None => FontData::from_static(CJK_FONT_BYTES),
    };
    defs.font_data.insert(CJK_FONT_NAME.to_string(), data.into());
    push_families(&mut defs);
    ctx.set_fonts(defs);
    log::info!("CJK fallback font installed: {CJK_FONT_NAME}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_font_spec_splits_face_index() {
        assert_eq!(
            parse_font_spec("path.ttc"),
            (PathBuf::from("path.ttc"), 0)
        );
        assert_eq!(
            parse_font_spec("path.ttc:2"),
            (PathBuf::from("path.ttc"), 2)
        );
        assert_eq!(
            parse_font_spec(" /usr/share/f/f.ttc:3 "),
            (PathBuf::from("/usr/share/f/f.ttc"), 3)
        );
    }

    #[test]
    fn parse_font_spec_keeps_non_numeric_suffix() {
        // A Windows drive colon and a junk suffix are not face indexes.
        assert_eq!(
            parse_font_spec("C:\\x\\font.ttf"),
            (PathBuf::from("C:\\x\\font.ttf"), 0)
        );
        assert_eq!(
            parse_font_spec("path:bad"),
            (PathBuf::from("path:bad"), 0)
        );
    }

    #[test]
    fn push_families_appends_last_and_idempotent() {
        let mut defs = FontDefinitions::default();
        push_families(&mut defs);
        for family in [FontFamily::Monospace, FontFamily::Proportional] {
            let list = &defs.families[&family];
            assert_eq!(list.last().map(String::as_str), Some(CJK_FONT_NAME));
        }
        push_families(&mut defs);
        let mono = &defs.families[&FontFamily::Monospace];
        let hits = mono
            .iter()
            .filter(|name| name.as_str() == CJK_FONT_NAME)
            .count();
        assert_eq!(hits, 1, "second push must not duplicate: {mono:?}");
    }
}

