# Cell backgrounds as merged runs (seam-free grid paint)

## Problem
`grid::draw_frame` painted one `rect_filled` PER CELL. epaint feathers
every rect edge, so same-colored neighbors (ANSI background runs,
inverse-video spans, full-width CJK runs) showed a lattice of faint
vertical/horizontal seams - measured 1/255 off per channel at the cell
pitch (9px columns at the default font) - and the sub-cell remainder
right of the last column (grid is floor(pane_w/cell_w) wide) rendered as
a wrong-colored pane-bg strip instead of extending the row's color.

## Design
- New `crates/app/src/render/bg_runs.rs` (pure functions, data-only
  `BgRun`): folds a frame's cell backgrounds into maximal same-color
  rectangles - inverse video swaps in the fg, explicit bg wins, `None`
  shows the pane default; wide (CJK) cells paint their color across both
  columns so their background has no holes; opacity (`with_opacity`) is
  applied once per run.
- `grid::draw_frame` paints the merged runs instead of per-cell rects;
  foreground (glyph ink, cursor) painting is unchanged.

## Fix
- `crates/app/src/render/bg_runs.rs` (new), `render/grid.rs`,
  `render/mod.rs` (module wiring).
- e2e `scripts/bin/e2e-bg-seams.sh`: scrot+PIL over a live Xvfb app -
  K1 vertical seams (every column of a full-width colored row within
  tolerance), K2 horizontal seams (2-row bands), K3 right-edge bleed
  (full-width runs reach pane_right-2), K4 CJK wide-cell bg holes.

## Verified
- `scripts/bin/e2e-bg-seams.sh` ALL GREEN (first live run, this tree):
  K1 0/1196 columns off >=2 for red/blue/orange bands, K2 0 rows off,
  K3 gap 0px, K4 zero pane-bg holes inside the magenta CJK run.
- `cargo fmt` / `clippy -D warnings` clean, `cargo test --workspace`
  321 passed / 0 failed, release build green (same session).
