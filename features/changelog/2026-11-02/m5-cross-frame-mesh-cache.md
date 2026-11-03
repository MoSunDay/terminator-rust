# M5: cross-frame mesh cache

## Problem
egui 0.36 calls `egui_ctx.tessellate(shapes, ppp)` on every delivered shape
every frame. M1's row cache removed shape GENERATION for unchanged rows, but
each row still held one `Shape::Text` per non-empty cell (300-900 per pane),
so every output frame re-tessellated all of them. egui explicitly rejects a
caller-side shape comparison cache (context.rs 2863-2865), so the cache has
to sit below shapes: on tessellated meshes.

## Change Summary
1. **`render/row_cache/mesh.rs` (new, 160 lines)**: `BakeKey` + `bake()`.
   Row layers cache `Vec<Arc<Mesh>>` in `render/row_cache.rs`; a cache hit
   adds the baked meshes to the painter directly, only a dirty row runs
   `tessellate` again.
2. **Bake key is the correctness boundary**: `StyleKey` (rect, cell,
   font_size, fill_alpha, `blink_on && has_blink`, ppp) PLUS `atlas_size`
   and the tessellation options. Glyph UVs are normalized by atlas size and
   feathering / pixel snapping are baked into vertices, so any of those
   changing drops every layer.
3. **Stable clip**: bake uses `clip_rect = Rect::EVERYTHING`; clipping stays
   a paint-time concern, independent of the pane's clip rect.
4. The cursor cell (block + cursor ink) remains an UNCACHED overlay: blink
   edges rebuild no row, a cursor move rebuilds exactly the two rows whose
   `hole` changed.

## Release-blocker fix (2026-11-03): atlas growth inside one `paint`
The first version captured `BakeKey::for_context` BEFORE `shapes.paint`.
Rows that introduced NEW glyphs grew the font atlas during layout, so the
pre-layout key normalized their UVs against the OLD atlas size; the rows
were cached as meshes and replayed verbatim with no repair, garbling the
last frame whenever the atlas crossed a texture size.

Fix:
- `Cache::sync_atlas` runs at paint entry; `Cache::end_frame` performs
  the same atlas probe at the frame boundary (catching growth from
  chrome glyphs painted after the last pane). The bake key is captured
  AFTER `grid::row_shapes`/`grid::bg_shapes` (the final size for that
  shape build).
- `Cache::end_frame` requests one corrective single-pass frame when the
  atlas size changed at the boundary. Limitation (one transitional
  frame): meshes baked earlier in the pass cannot be patched in-place -
  the boundary guard only requests the corrective pass, and the next
  frame's key comparison rebuilds every stale layer. No stale mesh
  survives to steady state.
- `RowEntry.built` distinguishes "never built" from "built at generation
  N" (a default generation alone would replay an empty entry).

Regression test: `atlas_growth_inside_paint_bakes_against_the_final_size`
forces 10 large hanzi in one row (atlas 32px -> 128px tall during paint)
and asserts per-vertex parity against a direct bake with the final key.

## Verification
- New vertex-equivalence tests in `render/row_cache/tests.rs`: cached paint
  path vs uncached `grid::frame_shapes`, compared per vertex (position, uv,
  color) for identical cells; changes to ppp / atlas size / tick / font size
  / fill alpha rebuild rows, equality does not.
- `cargo test --workspace` green (app 251); fmt clean; clippy ran with
  `-D warnings` and was green.
- `e2e-idle-cpu.sh` PASS (idle 0 frames / 0%, S14 main-thread 1.0% at 10Hz,
  S15 8-pane idle 0 frames) and `e2e-bg-seams.sh` stays green, so merged
  background runs and cursor cells are pixel-identical.
