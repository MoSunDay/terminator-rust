# 400-line rule: module splits + shared e2e lib

## Problem
Three workspace modules had outgrown the one-file 400-line hard limit
(`crates/app/src/input/resize.rs` 448, `render/bg_runs.rs` 564,
`crates/layout-tree/src/movepane.rs` 645), and the two heaviest Xvfb
e2e scripts (`e2e-dragdrop.sh`, `e2e-window-controls.sh`) duplicated
~550 lines of sandbox/Xvfb/cleanup/state-preset scaffolding.

## Design
Pure code motion, verified per module: the top-level `pub` API is
byte-identical (the new `mod.rs` re-exports each moved item with
`pub use`), unit tests relocate verbatim into sibling `tests.rs` files.
- `movepane/` splits by function boundary: `zone.rs` (zone_for /
  zone_rect), `surgery.rs` (detach/attach/swap leaf surgery),
  `sametab.rs` (move_pane_to_pane), `crosstab.rs`
  (move_pane_across_tabs), `tests.rs` (16 tests).
- `bg_runs/` splits by paint stage: `row.rs` (classify row colors),
  `merge.rs` (interval-absorbing run merge), `rect.rs` (edge bleed),
  `tests.rs` (12 tests).
- `resize/` splits by gesture stage: `hit.rs` (edge strips + dir_at),
  `gesture.rs` (resized_rect math), `strips.rs` (widget glue),
  `tests.rs` (13 tests).
- `scripts/bin/e2e-lib.sh` (318 lines): `fail`/`step`/`e2e_cleanup`,
  `e2e_sandbox` (HOME/XDG/sock/TERMINATOR_* env), `e2e_start_xvfb`
  (random display probe), state.json preset/assert helpers,
  chip-edge + overlay-fill pixel probes. Contract: the lib only
  DEFINES helpers (never touches shell options); anything that differs
  between scripts (geometry, shells, WM, launch) stays a parameter.
  Both scripts source it right after `set -euo pipefail`; dragdrop
  -386 lines, window-controls -141 lines, zero behavior change in the
  check sequence.

## Verified
- `cargo build/test --workspace` green (all unit tests relocated, none
  dropped); `cargo clippy --workspace --all-targets -- -D warnings`
  clean; `bash -n` on all three scripts.
- Longest new file: `movepane/tests.rs` 387 lines - every file back
  under the 400-line limit.
