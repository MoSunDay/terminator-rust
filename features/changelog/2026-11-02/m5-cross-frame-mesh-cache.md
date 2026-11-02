# M5: cross-frame mesh cache

## Problem
egui 0.36 calls `egui_ctx.tessellate(shapes, ppp)` on every delivered shape
every frame. M1's row cache removed shape GENERATION for unchanged rows, but
each row still held one `Shape::Text` per non-empty cell (300-900 per pane),
so every output frame re-tessellated all of them. egui explicitly rejects a
caller-side shape comparison cache (context.rs 2863-2865), so the cache has
to sit below shapes: on tessellated meshes.

## Change Summary
1. **`render/row_cache/mesh.rs` (new, 158 lines)**: `BakeKey` + `bake()`.
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

## Verification
- New vertex-equivalence tests in `render/row_cache/tests.rs`: cached paint
  path vs uncached `grid::frame_shapes`, compared per vertex (position, uv,
  color) for identical cells; changes to ppp / atlas size / tick / font size
  / fill alpha rebuild rows, equality does not.
- `cargo test --workspace` green (app 251); fmt clean; clippy baseline only.
- `e2e-idle-cpu.sh` PASS (idle 0 frames / 0%, S14 main-thread 1.0% at 10Hz,
  S15 8-pane idle 0 frames) and `e2e-bg-seams.sh` stays green, so merged
  background runs and cursor cells are pixel-identical.
