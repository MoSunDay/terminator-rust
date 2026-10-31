# Row-layered shape cache: one dirty row rebuilds one row

## Problem
The per-pane shape cache (render/shape_cache.rs, 2026-10-14) keyed a
pane's ENTIRE shape list on a pane-level `frame_gen` plus
`cursor_alpha`. Any output byte (a spinner's `\r`, one echoed key)
invalidated the whole pane's shapes: every non-empty cell re-ran
`LayoutJob::simple_singleline` (with a fresh `String`) before egui
re-tessellated the result. Worse, the cursor blink alpha sat IN the
key, so every blink edge (1-2/s on a focused pane, forever) rebuilt
ten thousand cells to recolor one.

## Change Summary
1. **`vt_pane::Frame::row_gens`** (term.rs): per-row generation,
   bumped exactly when a row's cells are rewritten (engine dirty-row
   patch, full rebuild, defensive blanking). Always >= 1 once built.
2. **`render/row_cache.rs`** (replaces shape_cache.rs): three layers -
   pane bg layer (pane fill + merged cell-bg runs, keyed on
   frame_gen), per-row ink layer (keyed on that row's own gen), and an
   uncached cursor overlay (block + cursor-cell ink, a few shapes).
   `StyleKey` = {rect, cell, font_size, fill_alpha, blink_on &&
   has_blink, pixels_per_point}; any change drops all layers.
3. **Cursor hoisting** (grid.rs): the cursor cell's ink is hoisted out
   of its row as a `hole` positioned by `cursor_cell` - alpha
   INDEPENDENT, so a blink edge rebuilds no row (the overlay recolors)
   and a cursor move rebuilds exactly two rows. With the block off the
   overlay repaints the holed cell's ink in its plain color (pixel
   parity with the row's own paint). `cursor_shapes` mirrors the row
   walk's wide-tail skip.
4. **`grid::frame_shapes`** (uncached preedit path, tests) = the
   composition bg + rows + overlay, pixel-identical to the cached
   path (e2e-bg-seams green).

## Verification (gl sandbox, release, e2e-idle-cpu.sh all PASS)
- S1 idle 0.0% / 0 frames kept; WAKE 10ms kept; S7 0.0% kept.
- S2 spinner pass_avg_us median 426 -> 315us (-26%), mean 683 -> 535.
- Interleaved A/B (3x each): total spinner CPU flat (267.3% vs
  265.7%) - remaining cost is llvmpipe raster + full tessellation +
  present (egui 0.36 has no cross-frame mesh cache; next lever).
- 467 unit tests; new row_cache contract tests + vt-pane row_gens
  semantics tests.
