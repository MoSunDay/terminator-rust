# Active-frame fixed overhead off the per-window path

## Problem
Every active frame paid the same fixed costs once per WINDOW PASS even
though they are frame-global: `ctx.input(|i| i.events.clone())`
allocated a fresh events Vec in pointer.rs AND keyboard.rs per pass
(large during mouse motion); `dividers()` re-walked the layout tree up
to three times per pass (hover probe, drag re-find, strip painter);
`measure_cells` re-ran a full `layout_no_wrap` galley probe per pass;
the chrome `mix` colors were recomputed at every paint call site; and
the global walkers (`auto_degrade`, `reconnect::pump`,
`ensure_sessions`, `close_exited`, `all_pane_ids`) re-ran in every
secondary viewport's pass for identical results, with `all_pane_ids`
itself O(n^2) in pane count (`Vec::contains` dedup) plus another
O(n^2) `ids.contains` in `ensure_sessions`.

## Change Summary
1. **Reused thread-local event buffers** (pointer.rs, keyboard.rs):
   one `ctx.input` read (pointer state merged into the same closure)
   `extend_from_slice`-ing into a take/put-back buffer; the drain loop
   and fn-exit restore keep it allocation-free in steady state. All
   early returns in `handle` predate the loop (only `continue` inside).
2. **Root-only global passes** (screen.rs): `auto_degrade` +
   `reconnect::pump` and `ensure_sessions` + `close_exited` + the
   window-removal guard now run under `viewport_id() == ROOT`, next to
   the existing session-bookkeeping block. The empty-window
   `do_new_tab` rescue stays per pass (it seeds the CURRENT window and
   spawns its session synchronously, so no ensure_sessions ordering
   dependency); the per-pane `win_id` recheck in the pane loop stays
   per pass (dead-pane clicks close windows during pointer routing).
3. **`all_pane_ids` dedup via `BTreeSet`** (state.rs) - sorted unique
   output, O(n log n); `ensure_sessions` builds one local `HashSet`
   for the stale filter (actions/mod.rs).
4. **Dividers computed once per pass** (screen.rs + mouse.rs):
   `divs` is built right before `divider_interaction` and shared with
   the strip painter; `find_divider_in(&[DividerHit], ..)` hit-tests
   the precomputed list (drag-on-a-non-active-tab falls back to a
   local recompute via `Cow`). The from-tree `find_divider` survives
   for tests only (`#[cfg(test)]`).
5. **Chrome color memo** (colors.rs): one single-slot thread-local
   keyed on the three palette inputs the mixes depend on
   (background/foreground/block_highlight) holding the whole `Chrome`
   bundle; the seven free functions are thin field reads.
6. **`measure_cells` memo** (grid.rs): single-slot keyed on
   (font_size, pixels_per_point); the fonts are installed once at
   startup (main.rs), so the probe result can never change mid-run.

## Verification
- 467 unit tests green (baseline count unchanged); release build clean.
- e2e-idle-cpu ALL PASS (idle 0 frames / 0.0% kept, spinner 49.1fps,
  busy 14.7fps); e2e-bg-seams ALL GREEN (pixel parity); e2e-empty-
  restore PASS (same-frame do_new_tab ordering); e2e-windows PASS;
  e2e-mouse-key PASS. e2e-cjk / e2e-edge-select / e2e-tab-close fail
  identically on the M1 baseline binary in this sandbox (no WM: the
  xdotool `windowactivate` step) - environmental, not regressions.
- Interleaved A/B vs M1 (3 rounds each): single-window spinner total
  CPU flat (274.5% vs 274.6%; pass_med ~185 vs ~183us) and mouse-
  motion flat (~460% llvmpipe-bound; pass_max_med 178-192 -> 165-185);
  3-window x 5-pane spinner flat after averaging a bimodal
  environmental state (round-mean pass_med 10147 vs 10132us). The
  macro numbers stay dominated by llvmpipe software raster + egui 0.36
  full re-tessellation (the upstream mesh-cache lever); this change
  removes fixed per-pass work that matters where raster is cheap.
- Harness fix found along the way: `/tmp/ab_spinner.sh` parsed the ctl
  `list` HEADER as the pane id ("ID"), so older "spinner" A/B runs
  measured ipc-wake repaints, not text output. Multi-window harness
  now uses `list --json`, per-window `xdotool windowfocus` + real keys
  (Ctrl+Shift+N/D/E), and targets the app by its unique binary path.
