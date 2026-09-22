//! Merged cell-background runs for the terminal grid.
//!
//! epaint feathers every `rect_filled` edge, so painting one rect per
//! cell leaves a lattice of faint seams between same-colored neighbors
//! (measured 1/255 off per channel at the cell pitch) plus
//! wrong-colored sub-cell strips where the grid ends short of the pane
//! edges (right and bottom). Pure functions here fold a frame's cell
//! backgrounds into maximal same-color rectangles; `grid::draw_frame`
//! paints those.

use egui::{Color32, Pos2, Rect, Vec2};
use vt_pane::{CellData, Frame as VtFrame};

use crate::render::colors::{to_c32, vt_rgb, with_opacity};
use crate::state::CellSize;

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
/// an equal color, vertically extended over their OVERLAP with same-
/// color runs of the previous row. Interval absorption, not exact
/// `(x, cols)` matching: run boundaries drift between adjacent rows
/// (partial-row selections, a band interrupted mid-row by a foreign
/// cell) and the exact-match rule left a feathered seam along every
/// misaligned row boundary. A prev run sticking out past the current
/// run is split into dead side pieces plus one live overlap piece that
/// spans the row boundary in a single rect; a current run spanning
/// several prev runs fills the uncovered gaps with fresh runs (their
/// top edge borders a different color above, which feathers like any
/// color boundary). Returns the indices of this row's runs (empty when
/// the row has none, which also breaks any vertical merge across
/// gaps).
fn merge_row(
    out: &mut Vec<BgRun>,
    prev: &[usize],
    y: u16,
    colors: &[Option<Color32>],
) -> Vec<usize> {
    // Snapshot the prev intervals BEFORE any split mutates `out`: a
    // later current run overlapping the SAME prev run must see its
    // original span. Only runs directly above (`y0 + rows == y`) absorb.
    let src: Vec<(u16, u16, usize)> = prev
        .iter()
        .map(|&i| (out[i].x, out[i].cols, out[i].y + out[i].rows, i))
        .filter(|&(_, _, end, _)| end == y)
        .map(|(x, cols, _, i)| (x, cols, i))
        .collect();
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
        let (s, e) = (start as u16, x as u16);
        let mut pos = s;
        for &(p0, pcols, pi) in src.iter() {
            let p1 = p0 + pcols;
            let (o0, o1) = (p0.max(pos), e.min(p1));
            if o0 >= o1 || out[pi].color != c {
                continue;
            }
            if p0 > pos {
                // columns above belong to another color or default
                cur.push(out.len());
                out.push(BgRun {
                    x: pos,
                    y,
                    cols: p0 - pos,
                    rows: 1,
                    color: c,
                });
            }
            split_extend(out, &mut cur, pi, o0, o1, y, c);
            pos = o1;
        }
        if pos < e {
            cur.push(out.len());
            out.push(BgRun {
                x: pos,
                y,
                cols: e - pos,
                rows: 1,
                color: c,
            });
        }
    }
    cur
}

/// Extend prev run `pi` downward over `[o0, o1)` - its overlap with the
/// current row's run - so the piece spans the row boundary in ONE rect
/// (no feathered seam lands on same-color columns). Parts of the prev
/// run sticking out past the overlap stay behind as dead side pieces
/// (their bottom edge borders a different color below).
fn split_extend(
    out: &mut Vec<BgRun>,
    cur: &mut Vec<usize>,
    pi: usize,
    o0: u16,
    o1: u16,
    y: u16,
    c: Color32,
) {
    let r = out[pi];
    let p1 = r.x + r.cols;
    if r.x == o0 && p1 == o1 {
        // fully inside the current run: extend in place
        out[pi].rows += 1;
        cur.push(pi);
        return;
    }
    if r.x < o0 {
        out[pi].cols = o0 - r.x; // left dead remainder keeps the slot
    } else {
        out[pi] = BgRun {
            // right dead remainder keeps the slot
            x: o1,
            y: r.y,
            cols: p1 - o1,
            rows: r.rows,
            color: c,
        };
    }
    if r.x < o0 && o1 < p1 {
        // sticking out on BOTH sides: the right remainder needs a slot
        out.push(BgRun {
            x: o1,
            y: r.y,
            cols: p1 - o1,
            rows: r.rows,
            color: c,
        });
    }
    cur.push(out.len());
    out.push(BgRun {
        x: o0,
        y: r.y,
        cols: o1 - o0,
        rows: y - r.y + 1,
        color: c,
    });
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
/// column/row bleed their right/bottom side to the pane edge so
/// full-width/full-height background regions cover the sub-cell
/// remainder (the grid is floor(pane/cell) cells wide/tall); the pane
/// clip rect guards any overshoot.
pub(crate) fn run_rect(
    pane: Rect,
    run: &BgRun,
    cell: CellSize,
    grid_cols: u16,
    grid_rows: u16,
) -> Rect {
    let left = pane.min.x + f32::from(run.x) * cell.w;
    let mut r = Rect::from_min_size(
        Pos2::new(left, pane.min.y + f32::from(run.y) * cell.h),
        Vec2::new(f32::from(run.cols) * cell.w, f32::from(run.rows) * cell.h),
    );
    if run.x + run.cols == grid_cols {
        r.max.x = r.right().max(pane.right());
    }
    if run.y + run.rows == grid_rows {
        r.max.y = r.bottom().max(pane.bottom());
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
    fn misaligned_rows_absorb_over_their_overlap() {
        // row0: red red BLUE red red; row1: red across all five. The
        // exact-match rule opened a fresh full-width run at row1 (a
        // seam along the whole boundary); absorption extends both red
        // runs over their overlap and only the column above the blue
        // cell starts fresh.
        let red = || CellData {
            text: " ".into(),
            bg: vtc(200, 20, 20),
            ..CellData::default()
        };
        let mut r0 = vec![red(), red()];
        r0.push(CellData {
            text: " ".into(),
            bg: vtc(20, 40, 220),
            ..CellData::default()
        });
        r0.push(red());
        r0.push(red());
        let r1 = vec![red(); 5];
        let rs = runs(&frame(5, 2, vec![r0, r1]));
        let got: Vec<(u16, u16, u16, u16)> =
            rs.iter().map(|r| (r.x, r.y, r.cols, r.rows)).collect();
        assert_eq!(
            got,
            vec![(0, 0, 2, 2), (2, 0, 1, 1), (3, 0, 2, 2), (2, 1, 1, 1)],
            "{rs:?}"
        );
    }

    #[test]
    fn sticking_out_prev_run_splits_dead_sides() {
        // row0: red x6; row1: red only over cols 2..5. The prev run
        // sticks out on both sides: the overlap piece spans the row
        // boundary, the side remainders stay one row tall.
        let red = || CellData {
            text: " ".into(),
            bg: vtc(200, 20, 20),
            ..CellData::default()
        };
        let r0 = vec![red(); 6];
        let mut r1 = vec![CellData::default(); 6];
        for cd in r1[2..5].iter_mut() {
            *cd = red();
        }
        let rs = runs(&frame(6, 2, vec![r0, r1]));
        let got: Vec<(u16, u16, u16, u16)> =
            rs.iter().map(|r| (r.x, r.y, r.cols, r.rows)).collect();
        assert_eq!(
            got,
            vec![(0, 0, 2, 1), (5, 0, 1, 1), (2, 0, 3, 2)],
            "{rs:?}"
        );
    }

    #[test]
    fn drifting_boundaries_chain_without_row_seams() {
        // A selection whose column window drifts every row: each row
        // boundary is crossed by a rect over the same-color overlap
        // (the exact-match rule re-opened a full-width run each row).
        let red = || CellData {
            text: " ".into(),
            bg: vtc(200, 20, 20),
            ..CellData::default()
        };
        let mut r0 = vec![CellData::default(); 3];
        r0[0] = red();
        r0[1] = red();
        let mut r1 = vec![CellData::default(); 3];
        r1[1] = red();
        r1[2] = red();
        let r2 = vec![red(); 3];
        let rs = runs(&frame(3, 3, vec![r0, r1, r2]));
        let got: Vec<(u16, u16, u16, u16)> =
            rs.iter().map(|r| (r.x, r.y, r.cols, r.rows)).collect();
        assert_eq!(
            got,
            vec![(0, 0, 1, 1), (1, 0, 1, 3), (2, 1, 1, 2), (0, 2, 1, 1)],
            "{rs:?}"
        );
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
    fn run_rect_bleeds_only_at_the_last_grid_column_and_row() {
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
        let r = run_rect(pane, &full, cell, 5, 3);
        assert_eq!(r.left(), 10.0);
        assert_eq!(r.top(), 38.0);
        assert_eq!(
            r.right(),
            pane.right(),
            "full-width runs bleed to the pane edge"
        );
        assert_ne!(r.width(), 45.0);
        assert_eq!(
            r.bottom(),
            pane.bottom(),
            "full-height runs bleed to the pane bottom edge"
        );
        assert_ne!(r.height(), 36.0);

        // one row short of the grid: no bottom bleed
        let tall = BgRun {
            x: 0,
            y: 0,
            cols: 5,
            rows: 2,
            color: Color32::RED,
        };
        assert_eq!(run_rect(pane, &tall, cell, 5, 3).bottom(), 56.0);

        // interior runs stop exactly at cols*cell.w
        let mid = BgRun {
            x: 1,
            y: 0,
            cols: 3,
            rows: 1,
            color: Color32::RED,
        };
        assert_eq!(run_rect(pane, &mid, cell, 5, 3).width(), 27.0);

        // one column short of the grid: no bleed
        let short = BgRun {
            x: 0,
            y: 0,
            cols: 4,
            rows: 1,
            color: Color32::RED,
        };
        assert_eq!(run_rect(pane, &short, cell, 5, 3).right(), 46.0);
    }
}
