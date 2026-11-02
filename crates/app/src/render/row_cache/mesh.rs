//! Bake cached shapes into GPU-ready meshes.
//!
//! Shape caching alone still pushes every `Shape::Text` through epaint's
//! tessellator on every frame: eframe calls `egui_ctx.tessellate(shapes,
//! ppp)` unconditionally, and a terminal row carries one text shape per
//! non-empty cell. Baking a row/background ONCE into an `epaint::Mesh`
//! turns that per-frame cost into a vertex memcpy (egui re-appends the
//! mesh, it does not re-lay-out glyphs).
//!
//! The bake key is part of [`super::StyleKey`]: glyph UVs are normalized
//! against the font atlas size, and feathering/dpi snapping are baked
//! into the vertices too. Atlas growth (new glyphs), a tessellation
//! option change or a monitor move MUST invalidate every layer - there
//! is no way to patch a baked mesh afterwards.
//!
//! Pure functions over plain data (repo style): the cache hands shapes
//! in, gets meshes back.

use std::sync::Arc;

use egui::epaint::{Mesh, TessellationOptions, Tessellator};
use egui::{Context, Rect, Shape};

/// Inputs of a bake beyond the shapes themselves. `pixels_per_point`
/// lives on the enclosing [`super::StyleKey`] (it also drives layout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BakeKey {
    /// Font atlas size at bake time: text UVs are normalized by it.
    pub atlas_size: [usize; 2],
    /// Feathering / pixel snapping / culling options at bake time.
    pub options: TessellationOptions,
}

impl BakeKey {
    /// Snapshot the current context's bake inputs.
    pub fn for_context(ctx: &Context) -> Self {
        Self {
            atlas_size: ctx.fonts(|f| f.font_image_size()),
            options: ctx.tessellation_options(|o| *o),
        }
    }
}

/// Bake `shapes` into one `Arc<Mesh>` per texture run, preserving order.
///
/// Culling is disabled while baking (`clip_rect = EVERYTHING`): a mesh
/// is clipped at paint time by the layer's `ClippedShape`, and baking
/// against one frame's clip rect would permanently drop shapes when a
/// later frame has a different (smaller) clip. Only textures present in
/// the input appear in the output; empty meshes are dropped.
pub fn bake(pixels_per_point: f32, key: BakeKey, shapes: Vec<Shape>) -> Vec<Arc<Mesh>> {
    let mut tess = Tessellator::new(pixels_per_point, key.options, key.atlas_size, Vec::new());
    tess.set_clip_rect(Rect::EVERYTHING);
    let mut out: Vec<Mesh> = Vec::new();
    for shape in shapes {
        let texture = shape.texture_id();
        if out.last().is_none_or(|m| m.texture_id != texture) {
            out.push(Mesh::with_texture(texture));
        }
        if let Some(mesh) = out.last_mut() {
            tess.tessellate_shape(shape, mesh);
        }
    }
    out.retain(|m| !m.is_empty());
    out.into_iter().map(Arc::new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::ClippedShape;
    use egui::{Color32, FontId, Pos2};

    fn context() -> Context {
        let ctx = Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        ctx
    }

    fn glyph(ctx: &Context) -> Shape {
        egui::Shape::galley(
            Pos2::ZERO,
            ctx.fonts_mut(|f| {
                f.layout_no_wrap("A".to_owned(), FontId::monospace(15.0), Color32::WHITE)
            }),
            Color32::WHITE,
        )
    }

    /// Baking is equivalent to letting egui tessellate the same shapes:
    /// same vertices (position/uv/color) in the same order.
    #[test]
    fn baked_mesh_matches_direct_tessellation() {
        let ctx = context();
        let key = BakeKey::for_context(&ctx);
        let shapes = vec![
            glyph(&ctx),
            Shape::line_segment(
                [Pos2::new(0.0, 18.0), Pos2::new(90.0, 18.0)],
                egui::Stroke::new(1.0, Color32::WHITE),
            ),
        ];
        let baked = bake(1.0, key, shapes.clone());
        let reference = Tessellator::new(1.0, key.options, key.atlas_size, Vec::new())
            .tessellate_shapes(
                shapes
                    .into_iter()
                    .map(|shape| ClippedShape {
                        clip_rect: Rect::EVERYTHING,
                        shape,
                    })
                    .collect(),
            );
        let vertices: Vec<_> = baked
            .iter()
            .flat_map(|m| m.vertices.iter().copied())
            .collect();
        let want: Vec<_> = reference
            .iter()
            .flat_map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(vertices.len(), want.len(), "vertex count");
        for (a, b) in vertices.iter().zip(want.iter()) {
            assert!(
                (a.pos - b.pos).length() < 1e-4,
                "pos {:?} vs {:?}",
                a.pos,
                b.pos
            );
            assert!((a.uv - b.uv).length() < 1e-5, "uv {:?} vs {:?}", a.uv, b.uv);
            assert_eq!(a.color, b.color);
        }
    }

    /// A null bake (e.g. an empty row) yields no meshes, not an empty
    /// draw call; mixed textures split into ordered runs.
    #[test]
    fn empty_shapes_and_texture_runs() {
        let ctx = context();
        let key = BakeKey::for_context(&ctx);
        assert!(bake(1.0, key, Vec::new()).is_empty());
        let white = egui::TextureId::default();
        let other = egui::TextureId::User(7);
        let mut mesh = Mesh::with_texture(other);
        mesh.vertices.push(egui::epaint::Vertex {
            pos: Pos2::ZERO,
            uv: Pos2::ZERO,
            color: Color32::WHITE,
        });
        let baked = bake(1.0, key, vec![glyph(&ctx), Shape::Mesh(mesh.into())]);
        assert_eq!(baked.len(), 2);
        assert_eq!(baked[0].texture_id, white);
        assert_eq!(baked[1].texture_id, other);
        assert!(!baked[0].vertices.is_empty() && !baked[1].vertices.is_empty());
    }
}
