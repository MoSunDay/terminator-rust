//! Merged cell-background runs for the terminal grid.
//!
//! epaint feathers every `rect_filled` edge, so painting one rect per
//! cell leaves a lattice of faint seams between same-colored neighbors
//! (measured 1/255 off per channel at the cell pitch) plus
//! wrong-colored sub-cell strips where the grid ends short of the pane
//! edges (right and bottom). Pure functions here fold a frame's cell
//! backgrounds into maximal same-color rectangles; `grid::draw_frame`
//! paints those.
//!
//! Split by responsibility: `row` (run data model + per-row
//! classification), `merge` (interval-absorbing merge + the frame
//! driver), `rect` (edge-bleed geometry + cursor ink).

mod merge;
mod rect;
mod row;

pub(crate) use merge::bg_runs;
pub(crate) use rect::{cursor_ink, run_rect};
pub(crate) use row::BgRun;

#[cfg(test)]
mod tests;
