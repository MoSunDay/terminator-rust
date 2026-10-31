//! Row-cache contract tests: synthetic VT frames drive rebuild counts.

use egui::{Color32, Pos2, Rect, Vec2};

use super::{Cache, StyleKey};
use crate::render::grid::DrawArgs;
use crate::state::CellSize;
use theme::builtin::dracula;
use vt_pane::term::{Color, FrameCursor};
use vt_pane::{CellData, Frame as VtFrame};

fn cell() -> CellSize {
    CellSize {
        w: 9.0,
        h: 18.0,
        wide_size: 15.0,
        w_px: 9,
        h_px: 18,
    }
}

fn style() -> StyleKey {
    StyleKey {
        rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(90.0, 90.0)),
        cell: cell(),
        font_size: 15.0,
        fill_alpha: 1.0,
        blink_on: true,
        pixels_per_point: 1.0,
    }
}

fn frame(rows: u16, gens: Vec<u64>) -> VtFrame {
    VtFrame {
        cols: 10,
        rows,
        cells: vec![vec![CellData::default(); 10]; rows as usize],
        row_gens: gens,
        cursor: FrameCursor {
            visible: true,
            ..FrameCursor::default()
        },
        default_fg: Color { r: 255, g: 255, b: 255 },
        ..VtFrame::default()
    }
}

fn args<'a>(pal: &'a theme::Palette, fr: &'a VtFrame, cursor_alpha: f32) -> DrawArgs<'a> {
    DrawArgs {
        fr,
        pal,
        cell: cell(),
        font_size: 15.0,
        cursor_alpha,
        blink_on: true,
        fill_alpha: 1.0,
    }
}

fn test_ui(name: &str) -> (egui::Context, egui::Ui) {
    let ctx = egui::Context::default();
    crate::ui::fonts::install(&ctx);
    ctx.begin_pass(egui::RawInput::default());
    let ui = egui::Ui::new(
        ctx.clone(),
        egui::Id::new(name),
        egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(90.0, 90.0))),
    );
    (ctx, ui)
}

fn drain(ctx: &egui::Context) {
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    ctx.begin_pass(egui::RawInput::default());
}

/// Two rows at gen 1, then only row 0 moves to gen 2: exactly one row
/// rebuild, row 1 replays.
#[test]
fn one_dirty_row_rebuilds_only_that_row() {
    let (ctx, ui) = test_ui("row-cache-dirty-row");
    let mut cache = Cache::default();
    let fr = frame(2, vec![1, 1]);
    cache.paint(&ctx, ui.painter(), 7, style(), 1, &args(&dracula(), &fr, 0.0));
    let after_first = cache.row_builds;
    let fr2 = frame(2, vec![2, 1]);
    cache.paint(&ctx, ui.painter(), 7, style(), 2, &args(&dracula(), &fr2, 0.0));
    assert_eq!(cache.row_builds - after_first, 1);
    // Nothing changed: zero rebuilds, pure replay.
    let before = cache.row_builds;
    cache.paint(&ctx, ui.painter(), 7, style(), 2, &args(&dracula(), &fr2, 0.0));
    assert_eq!(cache.row_builds, before);
    drain(&ctx);
}

/// The blink edge (alpha flip at a fixed cursor position) must rebuild
/// NO row: the cursor cell lives in the uncached overlay.
#[test]
fn blink_edge_rebuilds_no_rows() {
    let (ctx, ui) = test_ui("row-cache-blink-edge");
    let mut cache = Cache::default();
    let fr = frame(2, vec![1, 1]);
    cache.paint(&ctx, ui.painter(), 7, style(), 1, &args(&dracula(), &fr, 1.0));
    let before = cache.row_builds;
    // Same frame, alpha 1 -> 0 (blink off): rows replay, only the
    // overlay differs.
    cache.paint(&ctx, ui.painter(), 7, style(), 1, &args(&dracula(), &fr, 0.0));
    cache.paint(&ctx, ui.painter(), 7, style(), 1, &args(&dracula(), &fr, 1.0));
    assert_eq!(cache.row_builds, before, "blink edges must not rebuild rows");
    drain(&ctx);
}

/// A cursor move shifts the hole: the row it leaves and the row it
/// enters rebuild, every other row replays.
#[test]
fn cursor_move_rebuilds_source_and_destination_rows_only() {
    let (ctx, ui) = test_ui("row-cache-cursor-move");
    let mut cache = Cache::default();
    let mut fr = frame(3, vec![1, 1, 1]);
    fr.cursor = FrameCursor {
        x: 3,
        y: 0,
        visible: true,
        ..FrameCursor::default()
    };
    cache.paint(&ctx, ui.painter(), 7, style(), 1, &args(&dracula(), &fr, 1.0));
    let before = cache.row_builds;
    let mut fr2 = frame(3, vec![1, 1, 1]);
    fr2.cursor = FrameCursor {
        x: 3,
        y: 2,
        visible: true,
        ..FrameCursor::default()
    };
    cache.paint(&ctx, ui.painter(), 7, style(), 1, &args(&dracula(), &fr2, 1.0));
    assert_eq!(
        cache.row_builds - before,
        2,
        "hole leaves row 0 and enters row 2: rows 0 and 2 rebuild"
    );
    drain(&ctx);
}

/// A pane-level style change (resize, font, glass, scale) drops every
/// layer; the background also re-merges on any frame-generation change.
#[test]
fn style_change_rebuilds_all_rows_and_bg() {
    let (ctx, ui) = test_ui("row-cache-style-change");
    let mut cache = Cache::default();
    let fr = frame(2, vec![1, 1]);
    cache.paint(&ctx, ui.painter(), 7, style(), 1, &args(&dracula(), &fr, 0.0));
    let mut restyle = style();
    restyle.font_size = 17.0;
    cache.paint(&ctx, ui.painter(), 7, restyle, 1, &args(&dracula(), &fr, 0.0));
    assert_eq!(cache.row_builds, 4, "both rows rebuild under the new style");
    let bgs = cache.bg_builds;
    cache.paint(&ctx, ui.painter(), 7, restyle, 2, &args(&dracula(), &fr, 0.0));
    assert_eq!(cache.bg_builds, bgs + 1, "frame-gen change re-merges the bg");
    drain(&ctx);
}

/// Panes unpainted for two frames are pruned; a pane painted every
/// frame stays cached.
#[test]
fn end_frame_prunes_unpainted_panes() {
    let (ctx, ui) = test_ui("row-cache-prune");
    let mut cache = Cache::default();
    let fr = frame(1, vec![1]);
    cache.paint(&ctx, ui.painter(), 1, style(), 1, &args(&dracula(), &fr, 0.0));
    cache.end_frame();
    let before = cache.row_builds;
    cache.paint(&ctx, ui.painter(), 1, style(), 1, &args(&dracula(), &fr, 0.0));
    assert_eq!(cache.row_builds, before, "painting every frame keeps rows cached");
    cache.end_frame();
    cache.end_frame();
    cache.paint(&ctx, ui.painter(), 1, style(), 1, &args(&dracula(), &fr, 0.0));
    assert_eq!(cache.row_builds, before + 1, "an unpainted frame prunes the entry");
    drain(&ctx);
}

/// The overlay paints the cursor block at all (sanity: alpha > 0.02
/// draws a filled rect in the pass output, alpha 0 draws none).
#[test]
fn overlay_tracks_cursor_alpha() {
    let (ctx, ui) = test_ui("row-cache-overlay");
    let mut cache = Cache::default();
    let mut fr = frame(1, vec![1]);
    fr.cells[0][2] = CellData {
        text: "A".into(),
        ..CellData::default()
    };
    fr.cursor = FrameCursor {
        x: 2,
        y: 0,
        visible: true,
        ..FrameCursor::default()
    };
    cache.paint(&ctx, ui.painter(), 1, style(), 1, &args(&dracula(), &fr, 1.0));
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    assert!(
        out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT)),
        "visible cursor paints a block"
    );
    ctx.begin_pass(egui::RawInput::default());
    let ui2 = egui::Ui::new(
        ctx.clone(),
        egui::Id::new("row-cache-overlay-2"),
        egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(90.0, 90.0))),
    );
    cache.paint(&ctx, ui2.painter(), 1, style(), 1, &args(&dracula(), &fr, 0.0));
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    // Only the pane bg + row ink remain: no cursor-colored rect.
    assert!(
        !out.shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.corner_radius != egui::CornerRadius::ZERO && r.fill != Color32::TRANSPARENT)),
        "hidden cursor paints no block"
    );
}
