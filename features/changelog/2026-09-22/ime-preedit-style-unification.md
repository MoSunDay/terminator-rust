# IME preedit style unified with the rest of the app / desktop

## Problem
In terminal panes the IME composition (preedit) looked nothing like
everywhere else:
- The overlay (`render/preedit.rs`) invented its own style: a 2px
  ROUNDED accent pill (`palette.block_highlight`, purple) whose length
  was capped at 8 cells (longer compositions showed naked text), with
  the text laid out at the NARROW font size, top-left anchored, on
  natural advances - so a CJK composition rendered at a different
  size/height than the same text after commit (grid uses `wide_size`,
  per-cell pitch, LEFT_CENTER).
- Every other input surface in the app (rename editors, text fields =
  egui TextEdit) underlines composition with
  `visuals.ime_composition.active_underline_stroke` (2px square
  stroke), and desktop-wide IME preedit is "text + underline", not an
  accent pill.
- Candidate-window anchor: egui-winit forwards `IMEOutput.rect` to
  winit, and winit X11 consumes ONLY `rect.min` as the raw XIM spot
  (size ignored). We sent the cursor CELL rect, so the spot sat at the
  cell TOP-left - one full row above the X11 convention (xterm/GTK put
  the spot at the caret baseline), making the ibus candidate window
  float over the composing line instead of below it like every other
  app.

## Fix
- `render/preedit.rs` now paints with grid conventions: one glyph per
  cell on the snapped pitch (advance-2 glyphs classified via
  `Fonts::glyph_width` -> wide font + two-cell span), vertically
  centered like committed text, clipped to the pane rect; the
  underline is the shared egui composition stroke passed in by the
  caller (`ui.visuals().ime_composition.active_underline_stroke`),
  spanning the whole composition.
- `input/ime.rs` `anchor_rect()`: on X11 (`linux` and no
  `WAYLAND_DISPLAY`) `IMEOutput.rect.min` moves to the caret cell
  bottom-left (the XIM spot convention); `cursor_rect` stays the true
  cell for full-rect backends (macOS, Wayland).
- e2e `scripts/bin/e2e-ime.sh` M2 detector retuned to the stroke hue
  (blue signature `b-g>=18, r-g<=5, b>=110`; the violet focus ring has
  r-g=+22 and the oldWhite cursor block is red-leaning, so blink
  phase cannot fake the signal).

## Verification
- `cargo test -p app` (new tests: span classification 'M'/'汉',
  headless paint, anchor_rect baseline math) + workspace
  build/test/clippy/fmt green.
- `scripts/bin/e2e-ime.sh` ALL GREEN (M1-M4) on the real XIM chain;
  M2 sees 72 stroke pixels over a 0 baseline.
