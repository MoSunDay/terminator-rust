# Cell pitch quantization + default font 15 (font crispness)

Date: 2026-09-20 · Status: verified (unit + pixel measurement + 7 e2e)

## Problem
"字体很丑，字体的间隔也不对": on 1x-scale X11 the terminal text
rendered soft and letter spacing looked uneven, columns wobbled.

## Root cause
`grid::cell_rect` places cells at `x * cell.w` where cell.w was the
raw font advance - FRACTIONAL (Maple Mono 0.6em = 8.4pt at font 14).
epaint rasterizes unhinted glyphs at exact float positions, so every
column landed at a cycling subpixel phase (0, .4, .8, .2, .6, ...):
some glyphs crisp, some soft, apparent gaps alternating 8px/9px.
Measured: H onset deltas {8,9} stdev 0.49, Han {16,17,18}.

## Fix
- `measure_cells` snaps w/h to whole DEVICE pixels (`round(w*ppp)/ppp`)
  after the min guards; the wide scale derives from the snapped w, so
  a wide cell stays exactly 2 narrow cells (16px at 14pt).
- `DEFAULT_FONT_SIZE = 15.0` (was 14): 0.6em * 15 = 9.0px exactly ->
  naturally integer pitch, zero squeeze (20/25 are exact too; 14 snaps
  8.4 -> 8 with a 4.8% wide-glyph squeeze - the wide_size mechanism
  absorbs it). Persisted windows keep their font_size; snapping still
  applies at any size/ppp.
- Measured after: H deltas {9} stdev 0.0000, Han {18} stdev 0.0000,
  .9-.95 AA halo share 17.4% -> 9.8%, line pitch 20px.

## e2e
e2e-cjk.sh ink filters retuned for 15pt (wide/narrow split 10.5/11.5,
Han 12-15x13-14px, ASCII <=8 with one bold prompt rune at 9); all 7
suites + gates green (246 tests).
