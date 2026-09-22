# bg_runs interval-absorbing row merge + symmetric bottom bleed

Follow-up to the merged-runs paint (2026-09-22 `cell-bg-merged-runs.md`).

## Problem
Two residual seam shapes survived the merged-runs rewrite:
1. Row boundaries feathered whenever equal-color runs did not align on
   exact columns: `merge_row` matched prev-row runs by exact `(x, cols)`,
   so a full-width run above a row whose same-color run was split or
   shifted (selection spans, wide-cell reflow) painted as two rects
   meeting at a feathered horizontal edge - the same 1/255 lattice the
   per-cell paint had, just rarer.
2. The sub-cell remainder BELOW the last grid row (grid is
   floor(pane_h/cell_h) tall) showed the pane bg instead of extending a
   full-height column's color - the vertical mirror of the right-edge
   gap K3 had fixed, unfixed at the bottom.

## Design
- `merge_row` now INTERVAL-ABSORBS: a current-row run overlapping a
  same-color prev run extends it across the row boundary regardless of
  column alignment. `split_extend` splits the prev run around the
  overlap (inside / left-stick / right-stick / both-sticks cases) and
  keeps dead remainders in their original slots with their ORIGINAL
  `rows` (end == y -> can never absorb a later row); the live piece
  gets `rows = y - r.y + 1` and chains on. Prev intervals are
  snapshotted before mutation so later current runs see the original
  spans. Fresh (non-overlapping) runs only start where the column
  above is a different color or None - their top edge is a genuine
  color boundary, not a seam.
- `run_rect(grid_rows)` bleeds runs that reach the last grid ROW to
  the pane bottom, symmetric to the existing right bleed; `grid.rs`
  passes `fr.rows`. Rows past `cells.len()` are all-default (no runs),
  so only true full-height runs bleed.

## Fix
- `crates/app/src/render/bg_runs.rs`: `merge_row` + `split_extend`
  rewrite, `run_rect(grid_rows)`, 3 new unit tests (both-sides split,
  fresh-piece case) asserting exact `(x, y, cols, rows)` tuples.
- `crates/app/src/render/grid.rs`: 5-arg `run_rect` call; the bleed
  unit test now also asserts `bottom() == pane.bottom()`.
- `scripts/bin/e2e-bg-seams.sh`: K5 - full-grid red fill bottom-edge
  gate, self-calibrated against the pane-bg rect of the same scrot.
  Payload is BCE erase (`printf '\033[41m\033[H\033[2J'` sent as
  double-backslash text per the escaping rule), NEVER a wrapped
  spaces print: the vendored ghostty fast-fill path has a
  build-layout-dependent hang on multi-KB wrapped SGR-space fills
  ending mid-bottom-row (latent engine UB, documented in agents.md).

## Verified
- `scripts/bin/e2e-bg-seams.sh` K1-K5 ALL GREEN (three runs, incl. one
  on the exact commit tree): K1/K2 zero >=2-off deviations, K3 right
  gap 0px, K4 zero CJK bg holes, K5 bottom gap 0px.
- `cargo fmt` / `cargo clippy --workspace --all-targets -D warnings`
  clean; `cargo test --workspace` green (0 failed).
