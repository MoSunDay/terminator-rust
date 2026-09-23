# Pane-title font = terminal font; glyph-atomic selection over wide pairs

Two user-reported fixes.

## Problem
1. The pane header title (each terminal's title strip) rendered at
   `12.0 * scale` - it grew with the terminal font but never matched
   it (12pt at the 15pt default), while the tab chips had already
   moved to the terminal font (2026-09-22).
2. Selection highlight over CJK showed a thin vertical line through
   wide glyphs (English was seamless). Root cause: ghostty's
   `.selected` is a pure per-column test and the px->column floor
   math can land a selection boundary on the TAIL (spacer) column of
   a wide pair (or end it on the head). The bg fold then produced
   head = pane default + tail = selection color: two runs meeting
   exactly at the glyph's midline, feathered by epaint into the
   visible line.

## Fix
- `crates/app/src/ui/chrome.rs`: `Metrics.title_font` now EQUALS the
  terminal font size (like `chip_font`); doc + default-size test
  updated. `header_h` (24 * scale) already fits the larger text.
- `crates/app/src/render/bg_runs/row.rs`: third pass in `row_colors` -
  when either half of a wide pair is selected, BOTH columns paint the
  selection color (selection is glyph-atomic). Runs after the tail
  explicit-bg override so it also covers the mirror cut (selected
  head + explicitly-colored tail). The 2026-09-23 bg_runs split kept
  this in row.rs; two unit tests in `bg_runs/tests.rs`
  (`selection_starting_on_spacer_is_glyph_atomic`,
  `selected_wide_pair_beats_tail_explicit_bg`).

## Verified
- `cargo test -p app` 150 green; fmt/clippy clean.
- e2e-cjk, e2e-bg-seams (K1-K5), e2e-ui-style (incl. pane-title-center
  gate G and the font-20 scale gates) ALL GREEN.
- Live selection pixel probe (throwaway script): drag-select starting
  at 8 press offsets across a Han glyph's head/tail boundary - with
  the pass disabled, a 10px pane-default strip appears inside the
  glyph (press +4..+10 px); with the pass enabled, zero mid-glyph
  strips at every offset. Full-cell unselected strips (selection
  genuinely starting at the next glyph) correctly render plain.
