//! Run/row data model and per-row background classification.

use egui::Color32;
use vt_pane::CellData;

use crate::render::colors::{to_c32, vt_rgb};

/// One merged background rectangle: grid origin, size in cells, and the
/// (already opacity-adjusted) fill. Data-only.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BgRun {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
    pub color: Color32,
}

/// A cell's OPAQUE effective background incl. selection precedence:
/// selected > inverse video (fg as bg) > explicit bg; None = the pane
/// default shows (mirrors the bg half of `colors::cell_colors`).
pub(super) fn cell_color(cd: &CellData, default_fg: Color32, sel: Color32) -> Option<Color32> {
    if cd.selected {
        Some(sel)
    } else if cd.inverse {
        Some(cd.fg.map(vt_rgb).map_or(default_fg, to_c32))
    } else {
        cd.bg.map(vt_rgb).map(to_c32)
    }
}

/// Per-column OPAQUE backgrounds of one grid row (len == cols). A wide
/// cell's color fills both of its columns (clamped to `cols`); the tail
/// cell of a wide pair keeps its own explicit override for its column
/// only (the wide half must not swallow a colored tail). `sel` is the
/// OPAQUE selection color; selection is folded in via [`cell_color`] and
/// is glyph-atomic: a selection range can cut a wide pair in half (its
/// px->column floor math may start on the tail/spacer column or end on
/// the head), so either half selected paints BOTH columns `sel` - two
/// differently-colored halves would feather a vertical line through the
/// CJK char.
pub(super) fn row_colors(
    row: &[CellData],
    cols: u16,
    default_fg: Color32,
    sel: Color32,
) -> Vec<Option<Color32>> {
    let mut out = vec![None; cols as usize];
    let mut x = 0usize;
    while x < out.len() && x < row.len() {
        let cd = &row[x];
        let c = cell_color(cd, default_fg, sel);
        out[x] = c;
        if cd.wide {
            if x + 1 < out.len() {
                out[x + 1] = c;
            }
            x += 2;
        } else {
            x += 1;
        }
    }
    for x in 0..row.len().saturating_sub(1) {
        if row[x].wide && x + 1 < out.len() {
            if let Some(c) = cell_color(&row[x + 1], default_fg, sel) {
                out[x + 1] = Some(c);
            }
        }
    }
    // Glyph-atomic selection pass (last, so it also covers the tail's
    // explicit-bg override above): when either half of a wide pair is
    // selected the whole glyph is selected. Without this, a selection
    // starting on the spacer column leaves the head at the pane default
    // next to a selected tail - the run boundary lands exactly on the
    // glyph's midline and shows as a vertical line through the char.
    for x in 0..row.len().saturating_sub(1) {
        if row[x].wide && x + 1 < out.len() && (row[x].selected || row[x + 1].selected) {
            out[x] = Some(sel);
            out[x + 1] = Some(sel);
        }
    }
    out
}
