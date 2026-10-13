//! Per-pane egui shape cache: an unchanged terminal frame repaints from
//! a stored shape list instead of re-running the grid-to-shape walk.
//!
//! Pure data + free-standing functions over a plain struct (repo style):
//! the render pass looks a pane up by id and hands the builder closure
//! only on a miss.

use std::collections::HashMap;

use layout_tree::PaneId;

use crate::state::CellSize;

/// Cache identity of one pane's paint output. The VT frame's own
/// validity is covered by `frame_gen` (bumped by vt-pane on every pump
/// byte, resize, selection/viewport edit and theme sync), so an equal
/// key means pixel-identical output; the remaining fields are
/// everything `grid::frame_shapes` depends on beyond the VT frame
/// (geometry, cell metrics, font size, glass alpha, cursor fade and the
/// blink phase). `pixels_per_point` matters even when the point-space
/// inputs do not change: epaint lays out glyph vertices for the CURRENT
/// dpi (see `TextShape` - a galley must be rebuilt on a scale change),
/// and a monitor move can keep rect/cell identical while the scale
/// moves (macOS keeps the window's point size across screens).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    pub frame_gen: u64,
    pub rect: egui::Rect,
    pub cell: CellSize,
    pub font_size: f32,
    pub fill_alpha: f32,
    pub cursor_alpha: f32,
    pub blink_on: bool,
    pub pixels_per_point: f32,
}

/// One cached paint: the key it was built under plus the shapes.
struct Entry {
    key: Key,
    shapes: Vec<egui::Shape>,
    /// Frame stamp of the last `paint` hit (see [`Cache::end_frame`]).
    used: u64,
}

/// Shape list per pane, valid for the duration of one frame. Not
/// persisted, not per-window: pane ids are globally unique, so one map
/// serves every viewport pass of a frame.
#[derive(Default)]
pub struct Cache {
    map: HashMap<PaneId, Entry>,
    frame: u64,
}

impl Cache {
    /// Paint a pane's shapes through `painter`, building them first only
    /// on a key miss. Cloning the stored list is cheap: text galleys are
    /// `Arc`-wrapped inside `Shape` (the expensive vertex data is
    /// shared), and the rest are plain rects/strokes.
    pub fn paint(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        pane: PaneId,
        key: Key,
        build: impl FnOnce(&egui::Context) -> Vec<egui::Shape>,
    ) {
        if let Some(e) = self.map.get_mut(&pane) {
            if e.key == key {
                e.used = self.frame;
                painter.extend(e.shapes.iter().cloned());
                return;
            }
        }
        let shapes = build(ctx);
        painter.extend(shapes.iter().cloned());
        self.map.insert(
            pane,
            Entry {
                key,
                shapes,
                used: self.frame,
            },
        );
    }

    /// Close the frame: drop entries not painted this frame (closed or
    /// hidden panes) and stamp the next frame so fresh hits re-mark.
    pub fn end_frame(&mut self) {
        self.map.retain(|_, e| e.used == self.frame);
        self.frame += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ui(id: &str) -> (egui::Context, egui::Ui) {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        let ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new(id),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
        );
        (ctx, ui)
    }

    fn key() -> Key {
        Key {
            frame_gen: 7,
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(90.0, 54.0)),
            cell: CellSize {
                w: 9.0,
                h: 18.0,
                wide_size: 15.0,
                w_px: 9,
                h_px: 18,
            },
            font_size: 15.0,
            fill_alpha: 1.0,
            cursor_alpha: 0.0,
            blink_on: true,
            pixels_per_point: 1.0,
        }
    }

    fn rect_shape() -> egui::Shape {
        egui::Shape::rect_filled(key().rect, 0.0, egui::Color32::RED)
    }

    #[test]
    fn hit_replays_shapes_without_rebuild() {
        let (ctx, ui) = test_ui("shape-cache-hit");
        let mut cache = Cache::default();
        let mut builds = 0;
        cache.paint(&ctx, ui.painter(), 1, key(), |_| {
            builds += 1;
            vec![rect_shape()]
        });
        // Same key: the stored list replays, the builder stays uncalled.
        cache.paint(&ctx, ui.painter(), 1, key(), |_| {
            builds += 1;
            vec![rect_shape(), rect_shape()]
        });
        assert_eq!(builds, 1, "a key hit must not run the builder");
        let mut out = ctx.end_pass();
        // Clear BEFORE asserting: a dropped unapplied TexturesDelta aborts
        // the test process inside epaint's destructor.
        let delivered = out.shapes.len();
        out.textures_delta.clear();
        assert_eq!(
            delivered, 2,
            "both paints deliver (miss 1 shape, replayed hit 1 shape)"
        );
    }

    #[test]
    fn any_key_field_change_rebuilds() {
        let (ctx, ui) = test_ui("shape-cache-key");
        let mut cache = Cache::default();
        let mut builds = 0;
        let mut paint = |cache: &mut Cache, k: Key| {
            cache.paint(&ctx, ui.painter(), 1, k, |_| {
                builds += 1;
                vec![rect_shape()]
            });
        };
        fn bump(k: Key, f: impl FnOnce(&mut Key)) -> Key {
            let mut k = k;
            f(&mut k);
            k
        }
        paint(&mut cache, key());
        paint(&mut cache, bump(key(), |k| k.frame_gen += 1));
        paint(
            &mut cache,
            bump(key(), |k| {
                k.rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(45.0, 54.0));
            }),
        );
        paint(&mut cache, bump(key(), |k| k.blink_on = false));
        paint(&mut cache, bump(key(), |k| k.pixels_per_point = 2.0));
        assert_eq!(builds, 5, "frame_gen/rect/blink/scale changes must rebuild");
        let mut out = ctx.end_pass();
        let delivered = out.shapes.len();
        out.textures_delta.clear();
        assert_eq!(delivered, 5);
    }

    #[test]
    fn end_frame_prunes_unpainted_panes() {
        let (ctx, ui) = test_ui("shape-cache-prune");
        let mut cache = Cache::default();
        let builds = std::cell::Cell::new(0);
        let paint = |cache: &mut Cache, pane: PaneId| {
            cache.paint(&ctx, ui.painter(), pane, key(), |_| {
                builds.set(builds.get() + 1);
                vec![rect_shape()]
            });
        };
        paint(&mut cache, 1);
        cache.end_frame();
        // Pane 1 was painted this frame: the hit must NOT rebuild.
        paint(&mut cache, 1);
        assert_eq!(builds.get(), 1, "a pane painted every frame stays cached");
        // Skip a frame without painting: the stale entry is dropped.
        cache.end_frame();
        cache.end_frame();
        paint(&mut cache, 1);
        assert_eq!(builds.get(), 2, "an unpainted frame prunes the entry");
        let mut out = ctx.end_pass();
        let delivered = out.shapes.len();
        out.textures_delta.clear();
        // Three paints, three deliveries (every frame repaints the pane).
        assert_eq!(delivered, 3);
    }
}
