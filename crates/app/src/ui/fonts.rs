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

/// Check that `bytes` parses as the `index`-th font face via
/// `skrifa::FontRef::from_index` — the exact parser epaint runs in
/// `FontFace::new` (and `panic!`s on) at first layout. `Ok(())` means
/// epaint will accept the data; the error String carries the skrifa
/// `ReadError` Display for logging.
fn parse_ok(bytes: &[u8], index: u32) -> Result<(), String> {
    skrifa::FontRef::from_index(bytes, index)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Font data from `TERMINATOR_CJK_FONT` when the variable is set and the
/// file is readable AND parses as a valid font at the given face index;
/// `None` falls back to the embedded bytes. epaint panics on a bad
/// `FontDefinitions` entry, so the file is pre-validated here at startup
/// and an invalid one degrades to the embedded subset with a warning.
pub fn env_font_data() -> Option<FontData> {
    let spec = std::env::var("TERMINATOR_CJK_FONT").ok()?;
    if spec.trim().is_empty() {
        return None;
    }
    let (path, index) = parse_font_spec(&spec);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!(
                "cannot read TERMINATOR_CJK_FONT {}: {e}; using embedded font",
                path.display()
            );
            return None;
        }
    };
    if let Err(err) = parse_ok(&bytes, index) {
        log::warn!(
            "TERMINATOR_CJK_FONT {} (face {index}) is not a valid font: {err}; using embedded font",
            path.display()
        );
        return None;
    }
    log::info!("CJK font from env: {} face {index}", path.display());
    Some(FontData {
        index,
        ..FontData::from_owned(bytes)
    })
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
    fn parse_ok_accepts_embedded_font() {
        assert_eq!(parse_ok(CJK_FONT_BYTES, 0), Ok(()));
    }

    #[test]
    fn parse_ok_rejects_garbage_bytes() {
        let err = parse_ok(b"not a font at all", 0).expect_err("garbage must not parse");
        assert!(!err.is_empty(), "error should carry skrifa's Display");
    }

    #[test]
    fn parse_ok_rejects_bad_face_index() {
        // read-fonts' FontRef::from_index (what skrifa and epaint call)
        // returns InvalidCollectionIndex for a non-zero index on a
        // single (non-.ttc) font — it does NOT silently clamp or ignore
        // the index — so a bogus `:99` suffix falls back to the
        // embedded font instead of panicking in epaint.
        assert!(parse_ok(CJK_FONT_BYTES, 99).is_err());
    }

    #[test]
    fn parse_ok_rejects_empty_bytes() {
        // Belt-and-braces for the degenerate case: an empty file cannot
        // hold an sfnt header and must be rejected like any other junk.
        assert!(parse_ok(&[], 0).is_err());
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

