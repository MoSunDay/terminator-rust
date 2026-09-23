//! Run geometry with pane-edge bleed, and cursor-block ink.

use egui::{Color32, Pos2, Rect, Vec2};
use vt_pane::CellData;

use super::row::cell_color;
use super::BgRun;
use crate::state::CellSize;

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
