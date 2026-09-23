//! Interval-absorbing merge of row backgrounds into maximal runs.

use egui::Color32;
use vt_pane::Frame as VtFrame;

use super::row::row_colors;
use super::BgRun;
use crate::render::colors::with_opacity;

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
