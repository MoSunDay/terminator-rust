//! Merged cell-background runs for the terminal grid.
//!
//! epaint feathers every `rect_filled` edge, so painting one rect per
//! cell leaves a lattice of faint seams between same-colored neighbors
//! (measured 1/255 off per channel at the cell pitch) plus a
//! wrong-colored sub-cell strip where the grid ends short of the pane
//! edge. Pure functions here fold a frame's cell backgrounds into
//! maximal same-color rectangles; `grid::draw_frame` paints those.

use egui::{Color32, Pos2, Rect, Vec2};
use vt_pane::{CellData, Frame as VtFrame};

use crate::render::colors::{to_c32, vt_rgb, with_opacity};
use crate::state::CellSize;

/// One merged background rectangle: grid origin, size in cells, and the
/// (already opacity-adjusted) fill. Data-only.
#[derive(Debug)]
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
fn cell_color(cd: &CellData, default_fg: Color32, sel: Color32) -> Option<Color32> {
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
/// OPAQUE selection color; selection is folded in via [`cell_color`].
fn row_colors(
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
    out
}

/// Merge one row's column colors into `out`: maximal horizontal runs of
/// an equal color, vertically extended when the previous row ended an
/// identical run. Returns the indices of this row's runs (empty when the
/// row has none, which also breaks any vertical merge across gaps).
fn merge_row(
    out: &mut Vec<BgRun>,
    prev: &[usize],
    y: u16,
    colors: &[Option<Color32>],
) -> Vec<usize> {
    let mut cur: Vec<usize> = Vec::new();
    let mut x = 0usize;
    while x < colors.len() {
        let Some(c) = colors[x] else {
            x += 1;
            continue;
        };
        let start = x;
        while x < colors.len() && colors[x] == Some(c) {
            x += 1;
        }
        let cols = (x - start) as u16;
        if let Some(&i) = prev.iter().find(|&&i| {
            let r = &out[i];
            r.x == start as u16 && r.cols == cols && r.color == c && r.y + r.rows == y
        }) {
            out[i].rows += 1;
            cur.push(i);
        } else {
            cur.push(out.len());
            out.push(BgRun {
                x: start as u16,
                y,
                cols,
                rows: 1,
                color: c,
            });
        }
    }
    cur
}

/// All background runs of a frame merged into maximal rectangles.
/// `sel_bg` is the OPAQUE selection color, folded in per cell via
/// [`cell_color`] (no row cloning); every emitted fill is run through
/// `with_opacity(fill_alpha)` (uniform glass: cell bgs share the pane
/// fill alpha, ink stays opaque). Rows past `cells.len()` are
/// all-default.
pub(crate) fn bg_runs(
    fr: &VtFrame,
    default_fg: Color32,
    sel_bg: Color32,
    fill_alpha: f32,
) -> Vec<BgRun> {
    let mut out: Vec<BgRun> = Vec::new();
    let mut prev: Vec<usize> = Vec::new();
    for y in 0..fr.rows {
        let colors = match fr.cells.get(y as usize) {
            Some(row) => row_colors(row, fr.cols, default_fg, sel_bg)
                .into_iter()
                .map(|c| c.map(|c| with_opacity(c, fill_alpha)))
                .collect::<Vec<_>>(),
            None => vec![None; fr.cols as usize],
        };
        prev = merge_row(&mut out, &prev, y, &colors);
    }
    out
}

/// Glyph ink under the cursor block: the cell's own effective bg at FULL
/// alpha (a colored cell needs its own contrast color, not the pane bg),
/// else `bg_ink`. Alpha is dropped like `bg_ink` so the glyph stays
/// readable on glass.
pub(crate) fn cursor_ink(
    cd: &CellData,
    default_fg: Color32,
    sel_bg: Color32,
    bg_ink: Color32,
) -> Color32 {
    let c = cell_color(cd, default_fg, sel_bg);
    c.map_or(bg_ink, |c| Color32::from_rgb(c.r(), c.g(), c.b()))
}

/// Screen rect of a run inside `pane`. Runs reaching the last grid
/// column bleed their right side to the pane edge so full-width
/// background regions cover the sub-cell remainder; the pane clip rect
/// guards any overshoot.
pub(crate) fn run_rect(pane: Rect, run: &BgRun, cell: CellSize, grid_cols: u16) -> Rect {
    let left = pane.min.x + f32::from(run.x) * cell.w;
    let mut r = Rect::from_min_size(
        Pos2::new(left, pane.min.y + f32::from(run.y) * cell.h),
        Vec2::new(f32::from(run.cols) * cell.w, f32::from(run.rows) * cell.h),
    );
    if run.x + run.cols == grid_cols {
        r.max.x = r.right().max(pane.right());
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_pane::term::{Color as VtColor, FrameCursor};

    const FG: Color32 = Color32::from_rgb(248, 248, 242);
    const SEL: Color32 = Color32::from_rgb(68, 71, 90);

    fn vtc(r: u8, g: u8, b: u8) -> Option<VtColor> {
        Some(VtColor { r, g, b })
    }

    fn frame(cols: u16, rows: u16, cells: Vec<Vec<CellData>>) -> VtFrame {
        VtFrame {
            cols,
            rows,
            cursor: FrameCursor::default(),
            cells,
            ..Default::default()
        }
    }

    fn runs(fr: &VtFrame) -> Vec<BgRun> {
        bg_runs(fr, FG, SEL, 1.0)
    }

    #[test]
    fn uniform_rows_merge_into_one_run() {
        let row: Vec<CellData> = (0..5)
            .map(|_| CellData {
                text: " ".into(),
                bg: vtc(200, 20, 20),
                ..CellData::default()
            })
            .collect();
        let fr = frame(5, 3, vec![row.clone(), row.clone(), row]);
        let rs = runs(&fr);
        assert_eq!(rs.len(), 1, "3 identical rows -> one run: {rs:?}");
        assert_eq!((rs[0].x, rs[0].y, rs[0].cols, rs[0].rows), (0, 0, 5, 3));

        // rows past cells.len() stay default and break the vertical merge
        let one = frame(5, 3, vec![fr.cells[0].clone()]);
        let rs = runs(&one);
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].rows, 1);
    }

    #[test]
    fn two_colors_two_runs_and_defaults_skip() {
        let mut row = vec![
            CellData {
                text: " ".into(),
                bg: vtc(200, 20, 20),
                ..CellData::default()
            };
            3
        ];
        row.push(CellData::default());
        row.push(CellData {
            text: " ".into(),
            bg: vtc(20, 40, 220),
            ..CellData::default()
        });
        let rs = runs(&frame(5, 1, vec![row]));
        assert_eq!(rs.len(), 2, "{rs:?}");
        assert_eq!((rs[0].x, rs[0].cols), (0, 3));
        assert_eq!((rs[1].x, rs[1].cols), (4, 1));
    }

    #[test]
    fn wide_bg_spans_both_columns_but_tail_keeps_own() {
        let wide_red = CellData {
            text: "\u{6C49}".into(),
            wide: true,
            bg: vtc(200, 20, 20),
            ..CellData::default()
        };
        let mut row = vec![CellData::default(); 4];
        row[0] = wide_red.clone();
        let rs = runs(&frame(4, 1, vec![row]));
        assert_eq!(rs.len(), 1, "{rs:?}");
        assert_eq!((rs[0].x, rs[0].cols), (0, 2));

        // the tail cell's OWN bg wins its column only
        let mut row = vec![CellData::default(); 4];
        row[0] = wide_red;
        row[1] = CellData {
            text: " ".into(),
            bg: vtc(20, 40, 220),
            ..CellData::default()
        };
        let rs = runs(&frame(4, 1, vec![row]));
        assert_eq!(rs.len(), 2, "{rs:?}");
        assert_eq!((rs[0].x, rs[0].cols), (0, 1));
        assert_eq!((rs[1].x, rs[1].cols), (1, 1));
    }

    #[test]
    fn inverse_uses_fg_and_selection_overrides_all() {
        let mut row = vec![CellData::default(); 3];
        row[0] = CellData {
            text: "i".into(),
            inverse: true,
            fg: vtc(10, 200, 30),
            ..CellData::default()
        };
        row[2] = CellData {
            text: "s".into(),
            bg: vtc(1, 2, 3),
            selected: true,
            ..CellData::default()
        };
        let rs = runs(&frame(3, 1, vec![row]));
        assert_eq!(rs.len(), 2, "{rs:?}");
        assert_eq!(
            rs[0].color,
            Color32::from_rgb(10, 200, 30),
            "inverse: fg as bg"
        );
        assert_eq!(rs[1].color, SEL, "selection overrides the explicit bg");
    }

    #[test]
    fn cursor_ink_uses_cell_bg_at_full_alpha() {
        let plain = CellData::default();
        assert_eq!(
            cursor_ink(&plain, FG, SEL, Color32::from_rgb(40, 42, 54)),
            Color32::from_rgb(40, 42, 54)
        );
        let colored = CellData {
            bg: vtc(200, 20, 20),
            ..CellData::default()
        };
        assert_eq!(
            cursor_ink(&colored, FG, SEL, Color32::BLACK),
            Color32::from_rgb(200, 20, 20)
        );
        let sel = CellData {
            selected: true,
            ..CellData::default()
        };
        assert_eq!(cursor_ink(&sel, FG, SEL, Color32::BLACK), SEL);
    }

    #[test]
    fn run_rect_bleeds_only_at_the_last_grid_column() {
        let pane = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(49.0, 60.0));
        let cell = CellSize {
            w: 9.0,
            h: 18.0,
            wide_size: 15.0,
            w_px: 9,
            h_px: 18,
        };
        let full = BgRun {
            x: 0,
            y: 1,
            cols: 5,
            rows: 2,
            color: Color32::RED,
        };
        let r = run_rect(pane, &full, cell, 5);
        assert_eq!(r.left(), 10.0);
        assert_eq!(r.top(), 38.0);
        assert_eq!(r.height(), 36.0);
        assert_eq!(
            r.right(),
            pane.right(),
            "full-width runs bleed to the pane edge"
        );
        assert_ne!(r.width(), 45.0);

        // interior runs stop exactly at cols*cell.w
        let mid = BgRun {
            x: 1,
            y: 0,
            cols: 3,
            rows: 1,
            color: Color32::RED,
        };
        assert_eq!(run_rect(pane, &mid, cell, 5).width(), 27.0);

        // one column short of the grid: no bleed
        let short = BgRun {
            x: 0,
            y: 0,
            cols: 4,
            rows: 1,
            color: Color32::RED,
        };
        assert_eq!(run_rect(pane, &short, cell, 5).right(), 46.0);
    }
}
