use super::*;
use egui::{Color32, Pos2, Rect, Vec2};
use vt_pane::{CellData, Frame as VtFrame};

use crate::state::CellSize;
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
    let got: Vec<(u16, u16, u16, u16)> = rs.iter().map(|r| (r.x, r.y, r.cols, r.rows)).collect();
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
    let got: Vec<(u16, u16, u16, u16)> = rs.iter().map(|r| (r.x, r.y, r.cols, r.rows)).collect();
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
    let got: Vec<(u16, u16, u16, u16)> = rs.iter().map(|r| (r.x, r.y, r.cols, r.rows)).collect();
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
fn selection_starting_on_spacer_is_glyph_atomic() {
    // A selection range can START on the tail (spacer) column of a
    // wide pair (px->column floor math): without the glyph-atomic
    // pass the head stays at the pane default next to a selected
    // tail and the run boundary cuts the glyph in half.
    let wide = CellData {
        text: "\u{6C49}".into(),
        wide: true,
        ..CellData::default()
    };
    let mut row = vec![CellData::default(); 4];
    row[0] = wide;
    row[1].selected = true; // spacer column inside the selection
    row[2].selected = true;
    row[3].selected = true;
    let rs = runs(&frame(4, 1, vec![row]));
    assert_eq!(rs.len(), 1, "{rs:?}");
    assert_eq!((rs[0].x, rs[0].cols), (0, 4));
    assert_eq!(rs[0].color, SEL);
}

#[test]
fn selected_wide_pair_beats_tail_explicit_bg() {
    // Mirror cut: the range ENDS on the head column while the tail
    // carries its own explicit bg - the fixup override would split
    // the glyph; selection wins both columns.
    let wide_sel = CellData {
        text: "\u{6C49}".into(),
        wide: true,
        selected: true,
        ..CellData::default()
    };
    let mut row = vec![CellData::default(); 4];
    row[0] = wide_sel;
    row[1] = CellData {
        text: " ".into(),
        bg: vtc(20, 40, 220),
        ..CellData::default()
    };
    let rs = runs(&frame(4, 1, vec![row]));
    assert_eq!(rs.len(), 1, "{rs:?}");
    assert_eq!((rs[0].x, rs[0].cols), (0, 2));
    assert_eq!(rs[0].color, SEL);
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
