//! Per-pane layered shape cache: an unchanged terminal row replays its
//! stored shapes instead of re-running the grid-to-shape walk, so one
//! dirty row no longer rebuilds the whole pane.
//!
//! Three layers, three lifetimes:
//!  - the background layer (pane fill + merged cell-bg runs) is pane
//!    level and keyed on the frame generation - any visible change may
//!    re-merge the runs, but the walk is rect-only (no font layout);
//!  - the ink layer is one entry PER ROW keyed on that row's own
//!    generation (`vt_pane::Frame::row_gens`, bumped only when the row's
//!    cells were rewritten). The cursor cell's ink is hoisted out of the
//!    row (a `hole`) into the per-frame overlay below, so a cursor move
//!    rebuilds exactly two rows and a blink edge rebuilds none;
//!  - the cursor overlay (block + cursor-cell ink) is rebuilt every
//!    paint: a handful of shapes, and it is the only thing that varies
//!    with the blink alpha.
//!
//! Pure data + free-standing functions over a plain struct (repo style):
//! the render pass looks a pane up by id and hands `DrawArgs` in.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Context, Painter};
use layout_tree::PaneId;

pub use self::mesh::BakeKey;
use crate::render::grid::{self, DrawArgs};
use crate::state::CellSize;

mod mesh;

/// Pane-level style identity of a paint: everything `grid::row_shapes`
/// and `grid::bg_shapes` depend on beyond the VT frame's own per-row
/// content (geometry, cell metrics, font size, glass alpha, the blink
/// phase for blink-attribute text, and the dpi - epaint lays out glyph
/// vertices for the CURRENT pixels_per_point, and a monitor move can
/// keep rect/cell identical while the scale moves). Deliberately free
/// of `cursor_alpha`: that is overlay-only state, never a rebuild key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleKey {
    pub rect: egui::Rect,
    pub cell: CellSize,
    pub font_size: f32,
    pub fill_alpha: f32,
    pub blink_on: bool,
    pub pixels_per_point: f32,
    /// Mesh-bake inputs (font atlas size + tessellation options). Glyph
    /// UVs and feathering are baked into the cached vertices, so any
    /// change here must invalidate every layer.
    pub bake: BakeKey,
}

/// One cached row: the generation (and cursor-hole position) it was
/// built under, plus the meshes. `built` separates "never painted" from
/// every real generation value (`Option`), so a frame with MISSING row
/// generations can neither skip the first build nor falsely match a
/// fresh entry.
#[derive(Debug, Default)]
struct RowEntry {
    built: bool,
    gen: Option<u64>,
    hole: Option<u16>,
    meshes: Vec<Arc<egui::epaint::Mesh>>,
}

/// One pane's cached layers, valid across frames until their key moves.
struct PaneEntry {
    style: StyleKey,
    /// Background layer + the frame generation it was merged at.
    bg: Vec<Arc<egui::epaint::Mesh>>,
    bg_gen: Option<u64>,
    rows: Vec<RowEntry>,
    /// Frame stamp of the last `paint` hit (see [`Cache::end_frame`]).
    used: u64,
}

impl PaneEntry {
    fn new(style: StyleKey) -> Self {
        Self {
            style,
            bg: Vec::new(),
            bg_gen: None,
            rows: Vec::new(),
            used: 0,
        }
    }
}

/// Layer cache per pane id, valid for the process lifetime. Not
/// persisted, not per-window: pane ids are globally unique, so one map
/// serves every viewport pass of a frame.
#[derive(Default)]
pub struct Cache {
    map: HashMap<PaneId, PaneEntry>,
    frame: u64,
    /// Font atlas size seen at the last paint/frame boundary. A change
    /// means every baked mesh's text UVs are normalized against a stale
    /// atlas; [`Cache::sync_atlas`] / [`Cache::end_frame`] request one
    /// corrective pass, and the next frame's `StyleKey.bake` mismatch
    /// rebuilds every affected layer.
    atlas: Option<[usize; 2]>,
    /// Monotonic rebuild counters (tests, future frame-stats plumbing).
    pub(crate) row_builds: u64,
    pub(crate) bg_builds: u64,
}

impl Cache {
    /// Paint a pane's layers through `painter`, rebuilding only what its
    /// keys moved: stale rows, the background on a frame-generation
    /// change, and (always) the cheap cursor overlay.
    pub fn paint(
        &mut self,
        ctx: &Context,
        painter: &Painter,
        pane: PaneId,
        style: StyleKey,
        frame_gen: u64,
        a: &DrawArgs<'_>,
    ) {
        // Atlas growth during the previous frame (or an earlier pane of
        // this one) invalidates every baked mesh's text UVs. Never touch
        // a pane's stored `StyleKey` here: the next frame's bake-key
        // comparison must see the stale key and drop all affected
        // layers. This pass only guarantees that next frame HAPPENS.
        self.sync_atlas(ctx);
        let fr = a.fr;
        let now = self.frame;
        let e = self
            .map
            .entry(pane)
            .or_insert_with(|| PaneEntry::new(style));
        e.used = now;
        if e.style != style {
            // Every layer's inputs moved: drop them wholesale.
            e.style = style;
            e.rows.clear();
            e.bg.clear();
            e.bg_gen = None;
        }
        // Background first (it must sit under every row's ink).
        if e.bg_gen != Some(frame_gen) {
            let shapes = grid::bg_shapes(style.rect, a);
            // Read the key AFTER shape building: layout is what can add
            // glyphs to the atlas, and a key captured before it would
            // normalize UVs against the pre-layout atlas size.
            let bake = BakeKey::for_context(ctx);
            e.bg = mesh::bake(style.pixels_per_point, bake, shapes);
            e.bg_gen = Some(frame_gen);
            self.bg_builds += 1;
        }
        for m in &e.bg {
            painter.add(egui::Shape::Mesh(m.clone()));
        }
        // Per-row ink: replay unless that row's own gen or hole moved.
        if e.rows.len() != usize::from(fr.rows) {
            e.rows.resize_with(usize::from(fr.rows), RowEntry::default);
        }
        let at = grid::cursor_cell(fr);
        for y in 0..usize::from(fr.rows) {
            let hole = match at {
                Some((cx, cy)) if usize::from(cy) == y => Some(cx),
                _ => None,
            };
            let entry = &mut e.rows[y];
            // `built` makes a missing generation a never-match against a
            // FRESH entry without an in-band sentinel value (a sentinel
            // collided with `Frame::default()`'s generation and skipped
            // the first build). Rows with a gen still match exactly,
            // hole included.
            let gen = fr.row_gens.get(y).copied();
            if !entry.built || entry.gen != gen || entry.hole != hole {
                entry.built = true;
                entry.gen = gen;
                entry.hole = hole;
                let shapes = grid::row_shapes(ctx, style.rect, y, hole, a);
                // `row_shapes` can grow the atlas; bake against the size
                // the frame will actually upload.
                let bake = BakeKey::for_context(ctx);
                entry.meshes = mesh::bake(style.pixels_per_point, bake, shapes);
                self.row_builds += 1;
            }
            for m in &entry.meshes {
                painter.add(egui::Shape::Mesh(m.clone()));
            }
        }
        // Cursor overlay: never cached, never more than a few shapes.
        painter.extend(grid::cursor_shapes(ctx, style.rect, a));
    }

    /// Track the font atlas across paint calls. A size change requests
    /// ONE corrective single-pass frame: meshes baked earlier in this
    /// frame cannot be patched, but the next frame's `StyleKey.bake`
    /// comparison rebuilds every stale layer.
    pub fn sync_atlas(&mut self, ctx: &Context) {
        let now = ctx.fonts(|f| f.font_image_size());
        if self.atlas.is_some_and(|prev| prev != now) {
            crate::render::cadence::request_soon_single_pass(ctx);
        }
        self.atlas = Some(now);
    }

    /// Advance the frame stamp, catch atlas changes made after the last
    /// pane painted (chrome glyphs), and prune panes that went unpainted
    /// for two consecutive frames (a closed/hidden pane must not pin its
    /// shapes in memory).
    pub fn end_frame(&mut self, ctx: &Context) {
        let now = ctx.fonts(|f| f.font_image_size());
        if self.atlas.is_some_and(|prev| prev != now) {
            crate::render::cadence::request_soon_single_pass(ctx);
        }
        self.atlas = Some(now);
        self.frame = self.frame.wrapping_add(1);
        let prev = self.frame.wrapping_sub(1);
        self.map.retain(|_, e| e.used >= prev);
    }
}

#[cfg(test)]
#[path = "row_cache/tests.rs"]
mod tests;
