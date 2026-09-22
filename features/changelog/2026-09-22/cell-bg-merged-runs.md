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
  rectangles - precedence `cell_color`: selected > inverse video (fg as
  bg) > explicit bg > None (pane default); wide (CJK) cells paint their
  color across both columns so their background has no holes; opacity
  (`with_opacity`) is applied once per run. No row clones: selection is
  folded by precedence, not by rewriting the row.
- `grid::draw_frame` paints the merged runs instead of per-cell rects;
  runs that reach the last grid column bleed into the pane's right
  remainder (`run_rect`), so full-width regions look edge-to-edge.
- Cursor-block glyph ink now uses the CURSOR CELL's own effective bg at
  full alpha (`bg_runs::cursor_ink`), falling back to the pane bg -
  readable contrast on colored cells and on glass alike.

## Fix
- `crates/app/src/render/bg_runs.rs` (new), `render/grid.rs`,
  `render/mod.rs` (module wiring).
- e2e `scripts/bin/e2e-bg-seams.sh`: scrot+PIL over a live Xvfb app -
  K1 vertical seams (every column of a full-width colored row), K2
  horizontal seams (2-row bands), K3 right-edge bleed (full-width runs
  reach pane_right exactly), K4 CJK wide-cell bg holes. K1/K2 count only
  >=2-off deviations: the local NVIDIA/Vulkan present path dithers
  EXACTLY-1-off columns at screen-fixed positions on ANY fill (the
  single-rect pane bg included; GL/llvmpipe is clean) - that is GPU
  noise, not seams. Gate has teeth: on the pre-fix tree K1 fails (real
  seam columns) and K3 fails (remainder gap).

## Verified
- `scripts/bin/e2e-bg-seams.sh` ALL GREEN (first live run, this tree):
  K1 0/1196 columns off >=2 for red/blue/orange bands, K2 0 rows off,
  K3 gap 0px, K4 zero pane-bg holes inside the magenta CJK run.
- `cargo fmt` / `clippy -D warnings` clean, `cargo test --workspace`
  321 passed / 0 failed, release build green (same session).
