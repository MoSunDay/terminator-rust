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

use egui::{Context, Painter};
use layout_tree::PaneId;

use crate::render::grid::{self, DrawArgs};
use crate::state::CellSize;

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
}

/// One cached row: the generation (and cursor-hole position) it was
/// built under, plus the shapes. `gen` starts at the `u64::MAX`
/// sentinel so a fresh entry can never falsely match a real row gen.
#[derive(Debug, Default)]
struct RowEntry {
    gen: u64,
    hole: Option<u16>,
    shapes: Vec<egui::Shape>,
}

impl RowEntry {
    fn fresh() -> Self {
        Self {
            gen: u64::MAX,
            ..Self::default()
        }
    }
}

/// One pane's cached layers, valid across frames until their key moves.
struct PaneEntry {
    style: StyleKey,
    /// Background layer + the frame generation it was merged at.
    bg: Vec<egui::Shape>,
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
            e.bg = grid::bg_shapes(style.rect, a);
            e.bg_gen = Some(frame_gen);
            self.bg_builds += 1;
        }
        painter.extend(e.bg.iter().cloned());
        // Per-row ink: replay unless that row's own gen or hole moved.
        if e.rows.len() != usize::from(fr.rows) {
            e.rows.resize_with(usize::from(fr.rows), RowEntry::fresh);
        }
        let at = grid::cursor_cell(fr);
        for y in 0..usize::from(fr.rows) {
            let hole = match at {
                Some((cx, cy)) if usize::from(cy) == y => Some(cx),
                _ => None,
            };
            let entry = &mut e.rows[y];
            // A missing row gen is a defensive never-match (0 only in a
            // `Frame::default()`); rows with a gen must still match
            // exactly, hole included.
            let gen = fr.row_gens.get(y).copied().unwrap_or(u64::MAX);
            if entry.gen != gen || entry.hole != hole {
                entry.gen = gen;
                entry.hole = hole;
                entry.shapes = grid::row_shapes(ctx, style.rect, y, hole, a);
                self.row_builds += 1;
            }
            painter.extend(entry.shapes.iter().cloned());
        }
        // Cursor overlay: never cached, never more than a few shapes.
        painter.extend(grid::cursor_shapes(ctx, style.rect, a));
    }

    /// Advance the frame stamp and prune panes that went unpainted for
    /// two consecutive frames (a closed/hidden pane must not pin its
    /// shapes in memory).
    pub fn end_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        let prev = self.frame.wrapping_sub(1);
        self.map.retain(|_, e| e.used >= prev);
    }
}

#[cfg(test)]
#[path = "row_cache/tests.rs"]
mod tests;
