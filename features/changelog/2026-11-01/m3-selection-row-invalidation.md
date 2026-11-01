# M3: row-scoped selection invalidation

## Problem
Every selection edit (press / drag step / release / clear) ended in
`vtask::invalidate_frame` - a FULL cache drop. A drag over a 10k-cell
pane re-read every cell through the FFI per pointer event (tens of
thousands of calls per second of dragging), even though a drag can only
change the rows between its anchor and head.

Two engine facts blocked the obvious "let dirty rows do it" fix
(probe-verified against libghostty-vt):
- `set_selection` flips the engine's dirty tracking to `Dirty::Full`
  + all per-row flags on EVERY change (phantom dirt - no content moved).
- `PressEvent`/`DragEvent` expose no getters, so the anchor must be
  tracked vt-pane-side; `Selection` has `equals` but no coordinate
  accessors (per-row diffing would need snapshot machinery per event).

## Change Summary
1. **mouse.rs**: press/release stay FULL (one-shot, multi-click line
   select can span wrapped rows) but record the anchor row
   (`sel_anchor`) and span estimate (`sel_span`). `select_drag`:
   (a) `Selection::equals` against the installed selection - within-cell
   motion skips BOTH the engine write and the invalidation (zero work);
   (b) changed selections invalidate only the anchor..head viewport-row
   span (one row of slack per end for word/line snap), unioned with the
   previous span (a head can jump rows). `clear_selection` rebuilds the
   last span estimate; wheel/follow_output stay FULL.
2. **Scoping gate (task.rs, task/cache_policy.rs)**: the engine's dirty
   layers cannot arbitrate (phantom FULL), so freshness is tracked
   session-side - `content_epoch` bumped by every content-class
   invalidation, plus `engine_clean` (post-rebuild) / `sel_dirt`
   (only-phantom-dirt) flags. A span recorded at the current epoch may
   bypass and clear the engine layers in the rebuild; a stale span, a
   dirty engine at entry, or a pending FULL rebuild falls back to FULL.
3. **term.rs**: `snapshot_frame_into` takes the pending span; while it
   is fresh the engine dirty layers are ignored (and per-row flags are
   cleared for every row passed, so phantom flags cannot linger).
4. **task.rs split**: the cache-policy fns (frame_cached /
   bank_snapshot / invalidate_* / frame_gen) moved to
   `task/cache_policy.rs` (126 lines) to respect the 800-line limit;
   call paths unchanged via `pub use`.

## Verification (gl sandbox)
- `cargo test --workspace`: 474/0 (467 + 7 new in
  `vt-pane/src/tests/selection_rows.rs`): span-only row_gens bumps
  (rows outside [anchor-1, head+1] untouched), equals-skip (same-cell
  drag: no frame_gen move, no rebuild), union extension, pending-FULL
  dominance, clear-span rebuild, and the safety property - content
  arriving behind a pending span is NEVER hidden (all rows rebuild,
  cached cells cross-checked against the uncached fresh snapshot).
- clippy: no new warnings (drop_non_drop fixed); fmt clean.
- release build ok; e2e-idle-cpu PASS (idle 0 frames / 0.0%, wake
  29ms, spinner 51.7fps <= 65, busy 18.9fps); e2e-bg-seams ALL GREEN;
  e2e-mouse-key ALL PASSED.
- e2e-edge-select remains environmentally broken in this sandbox
  (clipboard init fails without a Wayland display; baseline-identical)
  - unit tests are the selection verification path.
