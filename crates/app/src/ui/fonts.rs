//! Embedded terminal fonts.
//!
//! A Julia Mono symbols subset (OFL, `assets/fonts/JuliaMono-OFL.txt`) is
//! appended AFTER the CJK fallback as the last resort: neither Maple, Noto
//! nor egui's built-in faces carry the supplemental arrows and symbols TUIs
//! emit (opencoder's subagent marker `⤷` U+2937, `≡`, `⤴`, `⭐` …), and
//! epaint renders every char no face supports as a literal `?`.
//!
//! Ships a Maple Mono Normal NF CN subset (OFL, see
//! `assets/fonts/MapleMono-OFL.txt`) registered as the FIRST entry of
//! the monospace and proportional family chains — ASCII, box drawing,
//! Nerd Font icons and the GB2312 Han set all render in it, and its CJK
//! advance is exactly 2x the latin one. A Noto Sans SC subset (OFL,
//! `assets/fonts/OFL.txt`) stays as the LAST entry so Hangul and rarer
//! glyphs Maple lacks still render. `TERMINATOR_FONT=path[:face_index]`
//! and `TERMINATOR_CJK_FONT=path[:face_index]` swap in system font
//! files (e.g. a multi-face `.ttc`) instead of the embedded bytes.

use std::path::PathBuf;

use egui::{FontData, FontDefinitions, FontFamily};

/// Registered name of the primary terminal font.
pub const MONO_FONT_NAME: &str = "terminator-mono";

/// Registered name of the CJK fallback font.
pub const CJK_FONT_NAME: &str = "terminator-cjk";

/// Registered name of the symbols last-resort font.
pub const SYM_FONT_NAME: &str = "terminator-symbols";

/// Embedded Maple Mono Normal NF CN subset (OFL).
const MONO_FONT_BYTES: &[u8] = include_bytes!("../../../../assets/fonts/MapleMonoNF-CN-subset.ttf");

/// Embedded Noto Sans SC subset (OFL).
const CJK_FONT_BYTES: &[u8] =
    include_bytes!("../../../../assets/fonts/NotoSansSC-Regular-subset.otf");

/// Embedded Julia Mono symbols subset (OFL) - arrows/math/misc symbols the
/// rest of the chain lacks.
const SYM_FONT_BYTES: &[u8] = include_bytes!("../../../../assets/fonts/TerminalSymbols-subset.ttf");

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

/// Font data from the env `var` (a `path[:face_index]` font spec) when
/// the variable is set and the file is readable AND parses as a valid
/// font at the given face index; `None` falls back to the embedded
/// bytes. epaint panics on a bad `FontDefinitions` entry, so the file is
/// pre-validated here at startup and an invalid one degrades to the
/// embedded subset with a warning.
fn env_font_data(var: &str) -> Option<FontData> {
    let spec = std::env::var(var).ok()?;
    if spec.trim().is_empty() {
        return None;
    }
    let (path, index) = parse_font_spec(&spec);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!(
                "cannot read {var} {}: {e}; using embedded font",
                path.display()
            );
            return None;
        }
    };
    if let Err(err) = parse_ok(&bytes, index) {
        log::warn!(
            "{var} {} (face {index}) is not a valid font: {err}; using embedded font",
            path.display()
        );
        return None;
    }
    log::info!("font from {var}: {} face {index}", path.display());
    Some(FontData {
        index,
        ..FontData::from_owned(bytes)
    })
}

/// Prepend the primary mono font to and append the CJK fallback + symbols
/// fonts to both family fallback chains. Idempotent: a second call must
/// not duplicate any entry.
fn push_families(defs: &mut FontDefinitions) {
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        let list = defs.families.entry(family).or_default();
        if !list.iter().any(|name| name == MONO_FONT_NAME) {
            list.insert(0, MONO_FONT_NAME.to_string());
        }
        if !list.iter().any(|name| name == CJK_FONT_NAME) {
            list.push(CJK_FONT_NAME.to_string());
        }
        if !list.iter().any(|name| name == SYM_FONT_NAME) {
            list.push(SYM_FONT_NAME.to_string());
        }
    }
}

/// Install the terminal font stack on the context: the embedded Maple
/// Mono subset first, the Noto Sans SC fallback last. Always succeeds
/// (the embedded bytes are compiled in); returns true for symmetry with
/// a future richer policy.
pub fn install(ctx: &egui::Context) -> bool {
    let mut defs = FontDefinitions::default();
    let mono =
        env_font_data("TERMINATOR_FONT").unwrap_or_else(|| FontData::from_static(MONO_FONT_BYTES));
    let cjk = env_font_data("TERMINATOR_CJK_FONT")
        .unwrap_or_else(|| FontData::from_static(CJK_FONT_BYTES));
    let sym = FontData::from_static(SYM_FONT_BYTES);
    defs.font_data
        .insert(MONO_FONT_NAME.to_string(), mono.into());
    defs.font_data.insert(CJK_FONT_NAME.to_string(), cjk.into());
    defs.font_data.insert(SYM_FONT_NAME.to_string(), sym.into());
    push_families(&mut defs);
    ctx.set_fonts(defs);
    log::info!("terminal fonts installed: {MONO_FONT_NAME} + {CJK_FONT_NAME} + {SYM_FONT_NAME}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_font_spec_splits_face_index() {
        assert_eq!(parse_font_spec("path.ttc"), (PathBuf::from("path.ttc"), 0));
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
        assert_eq!(parse_font_spec("path:bad"), (PathBuf::from("path:bad"), 0));
    }

    #[test]
    fn parse_ok_accepts_embedded_fonts() {
        assert_eq!(parse_ok(CJK_FONT_BYTES, 0), Ok(()));
        assert_eq!(parse_ok(MONO_FONT_BYTES, 0), Ok(()));
        assert_eq!(parse_ok(SYM_FONT_BYTES, 0), Ok(()));
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
    fn push_families_brackets_chain_and_idempotent() {
        let mut defs = FontDefinitions::default();
        push_families(&mut defs);
        for family in [FontFamily::Monospace, FontFamily::Proportional] {
            let list = &defs.families[&family];
            assert_eq!(list.first().map(String::as_str), Some(MONO_FONT_NAME));
            assert_eq!(list.last().map(String::as_str), Some(SYM_FONT_NAME));
            assert!(list.contains(&CJK_FONT_NAME.to_string()));
        }
        push_families(&mut defs);
        let mono = &defs.families[&FontFamily::Monospace];
        for name in [MONO_FONT_NAME, CJK_FONT_NAME, SYM_FONT_NAME] {
            let hits = mono.iter().filter(|n| n.as_str() == name).count();
            assert_eq!(hits, 1, "second push must not duplicate {name}: {mono:?}");
        }
    }
}
