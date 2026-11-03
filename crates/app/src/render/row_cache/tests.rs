//! Row-cache contract tests: synthetic VT frames drive rebuild counts.

use egui::{Color32, Pos2, Rect, Vec2};

use super::{Cache, StyleKey};
use crate::render::grid::{self, DrawArgs};
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

fn style(ctx: &egui::Context) -> StyleKey {
    StyleKey {
        rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(90.0, 90.0)),
        cell: cell(),
        font_size: 15.0,
        fill_alpha: 1.0,
        blink_on: true,
        pixels_per_point: 1.0,
        bake: super::BakeKey::for_context(ctx),
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
        default_fg: Color {
            r: 255,
            g: 255,
            b: 255,
        },
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
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    let after_first = cache.row_builds;
    let fr2 = frame(2, vec![2, 1]);
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        2,
        &args(&dracula(), &fr2, 0.0),
    );
    assert_eq!(cache.row_builds - after_first, 1);
    // Nothing changed: zero rebuilds, pure replay.
    let before = cache.row_builds;
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        2,
        &args(&dracula(), &fr2, 0.0),
    );
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
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 1.0),
    );
    let before = cache.row_builds;
    // Same frame, alpha 1 -> 0 (blink off): rows replay, only the
    // overlay differs.
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 1.0),
    );
    assert_eq!(
        cache.row_builds, before,
        "blink edges must not rebuild rows"
    );
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
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 1.0),
    );
    let before = cache.row_builds;
    let mut fr2 = frame(3, vec![1, 1, 1]);
    fr2.cursor = FrameCursor {
        x: 3,
        y: 2,
        visible: true,
        ..FrameCursor::default()
    };
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr2, 1.0),
    );
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
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    let mut restyle = style(&ctx);
    restyle.font_size = 17.0;
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        restyle,
        1,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(cache.row_builds, 4, "both rows rebuild under the new style");
    let bgs = cache.bg_builds;
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        restyle,
        2,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(
        cache.bg_builds,
        bgs + 1,
        "frame-gen change re-merges the bg"
    );
    drain(&ctx);
}

/// Panes unpainted for two frames are pruned; a pane painted every
/// frame stays cached.
#[test]
fn end_frame_prunes_unpainted_panes() {
    let (ctx, ui) = test_ui("row-cache-prune");
    let mut cache = Cache::default();
    let fr = frame(1, vec![1]);
    cache.paint(
        &ctx,
        ui.painter(),
        1,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    cache.end_frame(&ctx);
    let before = cache.row_builds;
    cache.paint(
        &ctx,
        ui.painter(),
        1,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(
        cache.row_builds, before,
        "painting every frame keeps rows cached"
    );
    cache.end_frame(&ctx);
    cache.end_frame(&ctx);
    cache.paint(
        &ctx,
        ui.painter(),
        1,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(
        cache.row_builds,
        before + 1,
        "an unpainted frame prunes the entry"
    );
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
    cache.paint(
        &ctx,
        ui.painter(),
        1,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 1.0),
    );
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    assert!(
        out.shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT)),
        "visible cursor paints a block"
    );
    ctx.begin_pass(egui::RawInput::default());
    let ui2 = egui::Ui::new(
        ctx.clone(),
        egui::Id::new("row-cache-overlay-2"),
        egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(90.0, 90.0))),
    );
    cache.paint(
        &ctx,
        ui2.painter(),
        1,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
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

/// A frame with NO row generations must still build every row on the
/// first paint (the old `u64::MAX` sentinel matched a fresh entry) and
/// replay it on the second.
#[test]
fn missing_row_generations_build_then_replay() {
    let (ctx, ui) = test_ui("row-cache-missing-gen");
    let mut cache = Cache::default();
    let fr = frame(3, vec![]);
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(cache.row_builds, 3, "first paint must build every row");
    let before = cache.row_builds;
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(
        cache.row_builds, before,
        "missing generations replay once built"
    );
    drain(&ctx);
}

/// An atlas-size change requests exactly one corrective single-pass
/// frame, and the pane's stored StyleKey stays stale so the next frame's
/// real bake key drops every row.
#[test]
fn atlas_change_requests_corrective_frame_and_rebuilds_stale_rows() {
    let (ctx, ui) = test_ui("row-cache-atlas-guard");
    let mut cache = Cache::default();
    let fr = frame(2, vec![1, 1]);
    let real = ctx.fonts(|f| f.font_image_size());
    let mut stale = style(&ctx);
    stale.bake.atlas_size = [
        real[0].saturating_mul(2).max(1),
        real[1].saturating_mul(2).max(1),
    ];
    // Prime as a pane painted BEFORE another pane grew the atlas: the
    // stored invalidation key is the pre-growth one.
    cache.paint(&ctx, ui.painter(), 7, stale, 1, &args(&dracula(), &fr, 0.0));
    drain(&ctx);
    // Pretend the atlas grew after the last paint of the frame.
    cache.atlas = Some(stale.bake.atlas_size);
    cache.end_frame(&ctx);
    assert!(
        ctx.has_requested_repaint(),
        "atlas growth must request a corrective frame"
    );
    let before = cache.row_builds;
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        2,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(
        cache.row_builds - before,
        2,
        "the real key differs from the stale stored key -> every row rebuilds"
    );
    drain(&ctx);
}

/// Real atlas growth inside one `paint`: the row that adds glyphs must
/// be baked against the FINAL atlas size, not the key captured before
/// layout. Vertex parity against a direct bake with the final key would
/// fail on every pre-fix UV.
#[test]
fn atlas_growth_inside_paint_bakes_against_the_final_size() {
    let (ctx, ui) = test_ui("row-cache-atlas-growth");
    let mut cache = Cache::default();
    let mut fr = frame(1, vec![1]);
    fr.cursor.visible = false;
    // Distinct GB2312 hanzi at a large size force the atlas to grow
    // while the row lays out.
    let chars: Vec<char> = "一二三四五六七八九十".chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        fr.cells[0][i] = CellData {
            text: ch.to_string(),
            ..CellData::default()
        };
    }
    let cell = CellSize {
        w: 64.0,
        h: 72.0,
        wide_size: 64.0,
        w_px: 64,
        h_px: 72,
    };
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 72.0));
    let mut st = style(&ctx);
    st.rect = rect;
    st.cell = cell;
    st.font_size = 60.0;
    let a = DrawArgs {
        fr: &fr,
        pal: &dracula(),
        cell,
        font_size: 60.0,
        cursor_alpha: 0.0,
        blink_on: true,
        fill_alpha: 1.0,
    };
    let before = ctx.fonts(|f| f.font_image_size());
    cache.paint(&ctx, ui.painter(), 7, st, 1, &a);
    let after = ctx.fonts(|f| f.font_image_size());
    assert_ne!(before, after, "test payload must grow the atlas");

    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    let got: Vec<egui::epaint::Vertex> = out
        .shapes
        .iter()
        .map(|c| &c.shape)
        .flat_map(|s| match s {
            egui::Shape::Mesh(m) => m.vertices.clone(),
            other => panic!("cached layer must deliver a mesh, got {other:?}"),
        })
        .collect();
    ctx.begin_pass(egui::RawInput::default());
    let final_key = super::BakeKey::for_context(&ctx);
    let mut want: Vec<egui::epaint::Vertex> = Vec::new();
    for m in super::mesh::bake(1.0, final_key, grid::bg_shapes(rect, &a)) {
        want.extend(m.vertices.iter().copied());
    }
    for m in super::mesh::bake(1.0, final_key, grid::row_shapes(&ctx, rect, 0, None, &a)) {
        want.extend(m.vertices.iter().copied());
    }
    assert_eq!(got.len(), want.len(), "cached vertex count");
    for (i, (a, b)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (a.pos - b.pos).length() < 1e-4
                && (a.uv - b.uv).length() < 1e-5
                && a.color == b.color,
            "vertex {i} diverged: got pos={:?} uv={:?} color={:?}; want pos={:?} uv={:?} color={:?}",
            a.pos,
            a.uv,
            a.color,
            b.pos,
            b.uv,
            b.color
        );
    }
    drain(&ctx);
}

/// Atlas growth and tessellation-option changes bake different vertices
/// (UV normalization / feathering) even when geometry and content are
/// identical: every layer must rebuild, and the painter must deliver
/// meshes (not shapes) for the cached layers.
#[test]
fn bake_key_change_rebuilds_all_rows_and_bg() {
    let (ctx, ui) = test_ui("row-cache-bake-key");
    let mut cache = Cache::default();
    let fr = frame(2, vec![1, 1]);
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        style(&ctx),
        1,
        &args(&dracula(), &fr, 0.0),
    );
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    assert!(
        out.shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Mesh(_))),
        "cached layers are delivered as meshes"
    );
    ctx.begin_pass(egui::RawInput::default());

    let mut restyle = style(&ctx);
    restyle.bake.atlas_size = [
        restyle.bake.atlas_size[0].max(1) * 2,
        restyle.bake.atlas_size[1].max(1) * 2,
    ];
    let rows = cache.row_builds;
    let bgs = cache.bg_builds;
    cache.paint(
        &ctx,
        ui.painter(),
        7,
        restyle,
        1,
        &args(&dracula(), &fr, 0.0),
    );
    assert_eq!(cache.row_builds - rows, 2, "atlas growth rebuilds rows");
    assert_eq!(cache.bg_builds - bgs, 1, "atlas growth rebuilds bg");
    drain(&ctx);

    let mut opts = restyle;
    opts.bake.options.feathering = !opts.bake.options.feathering;
    let rows = cache.row_builds;
    cache.paint(&ctx, ui.painter(), 7, opts, 1, &args(&dracula(), &fr, 0.0));
    assert_eq!(cache.row_builds - rows, 2, "tess options rebuild rows");
    drain(&ctx);
}
