//! Terminal Frame -> egui painter grid drawing.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use theme::Palette;
use vt_pane::Frame as VtFrame;

use crate::render::bg_runs;
use crate::render::colors::{cell_colors, to_c32, vt_rgb, with_opacity};
use crate::state::CellSize;

/// Measure the monospace cell metrics from egui fonts.
pub fn measure_cells(ctx: &egui::Context, font_size: f32) -> CellSize {
    let font = FontId::monospace(font_size);
    let ppp = ctx.pixels_per_point();
    let (w, h, wide_advance) = ctx.fonts_mut(|f| {
        let (w, h) = (f.glyph_width(&font, 'M'), f.row_height(&font));
        // Fullwidth probe through the same family fallback chain: with
        // the embedded CJK font installed this resolves to a Han glyph.
        let wide = f
            .layout_no_wrap("\u{6C49}".to_owned(), font.clone(), Color32::WHITE)
            .rect
            .width();
        (w, h, wide)
    });
    let w = w.max(4.0);
    let h = h.max(6.0);
    // A fractional cell pitch (Maple 0.6em advance = 8.4pt at font 14)
    // puts every column at a cycling subpixel phase (0,.4,.8,.2,.6,...):
    // glyphs rasterize unhinted at those offsets, so ink goes soft and
    // apparent letter spacing wobbles per column at pixels_per_point 1.0.
    // Quantize the PITCH to whole device pixels; the glyph size itself
    // stays untouched (hinting/acl rounds it independently).
    let w = (w * ppp).round().max(1.0) / ppp;
    let h = (h * ppp).round().max(1.0) / ppp;
    // A wide cell must span exactly two narrow cells: derive the paint
    // size by scaling the probe advance to the SNAPPED 2*w. Degenerate
    // measurements (no CJK glyph found, zero advance) fall back to a
    // sane 1.2x.
    let scale = if wide_advance > f32::EPSILON {
        2.0 * w / wide_advance
    } else {
        0.0
    };
    let wide_size = if scale.is_finite() && (0.8..=1.8).contains(&scale) {
        font_size * scale
    } else {
        font_size * 1.2
    };
    CellSize {
        w,
        h,
        wide_size,
        w_px: (w * ppp).round().max(1.0) as u32,
        h_px: (h * ppp).round().max(1.0) as u32,
    }
}

/// layout_tree::Rect -> egui::Rect.
pub fn egui_rect(r: layout_tree::Rect) -> Rect {
    Rect::from_min_size(Pos2::new(r.x, r.y), Vec2::new(r.w, r.h))
}

/// egui::Rect -> layout_tree::Rect.
pub fn lt_rect(r: Rect) -> layout_tree::Rect {
    layout_tree::Rect {
        x: r.min.x,
        y: r.min.y,
        w: r.width(),
        h: r.height(),
    }
}

fn cell_rect(origin: Rect, x: u16, y: u16, cell: CellSize, span: u16) -> Rect {
    Rect::from_min_size(
        Pos2::new(
            origin.min.x + f32::from(x) * cell.w,
            origin.min.y + f32::from(y) * cell.h,
        ),
        Vec2::new(cell.w * f32::from(span), cell.h),
    )
}

/// Focused-pane cursor cell rect (the IME anchor), None while the cursor
/// is hidden or outside the grid (same guard as the drawn cursor block).
pub fn cursor_rect(origin: Rect, fr: &VtFrame, cell: CellSize) -> Option<Rect> {
    if fr.cursor.visible && fr.cursor.x < fr.cols && fr.cursor.y < fr.rows {
        Some(cell_rect(origin, fr.cursor.x, fr.cursor.y, cell, 1))
    } else {
        None
    }
}

/// Bundled arguments of [`draw_frame`] (keeps the call sites readable).
pub struct DrawArgs<'a> {
    pub fr: &'a VtFrame,
    pub pal: &'a Palette,
    pub cell: CellSize,
    pub font_size: f32,
    /// Cursor visibility alpha (0 = hidden, 1 = solid block).
    pub cursor_alpha: f32,
    /// Opaque effective pane background (theme or global override).
    pub bg: Color32,
    /// Pane fill alpha: `pane_bg_alpha(settings.transparency,
    /// settings.opacity)` (pane bg + cell bgs), computed by the caller.
    pub fill_alpha: f32,
}

/// Draw one terminal snapshot into `rect`.
///
/// Background first (pane effective bg), then the cell backgrounds as
/// MERGED same-color runs (see [`crate::render::bg_runs`]: one feathered
/// edge per region instead of a per-cell seam lattice), the cursor
/// block, text, and underlines. Wide cells span two columns and the
/// trailing empty cell after them is skipped (per row: a row-final wide
/// cell must not eat the next row's first cell).
pub fn draw_frame(painter: &Painter, rect: Rect, a: &DrawArgs<'_>) {
    let fr = a.fr;
    let pal = a.pal;
    let cell = a.cell;
    let font_size = a.font_size;
    let cursor_alpha = a.cursor_alpha;
    let fill_alpha = a.fill_alpha;
    let bg = with_opacity(a.bg, fill_alpha);
    // Cursor-block glyph ink uses the bg COLOR at full alpha: on glass the
    // semi-transparent fill would render the glyph invisible.
    let bg_ink = Color32::from_rgb(bg.r(), bg.g(), bg.b());
    painter.rect_filled(rect, 0.0, bg);
    let default_fg = to_c32(pal.foreground);
    let cursor_col = fr
        .cursor_color
        .map(vt_rgb)
        .map_or(to_c32(pal.cursor), to_c32);
    // Soft blink: the block fades between the pane bg and the cursor
    // color (alpha 1 = legacy solid block). Peak capped at 0.9 so even
    // the fully-on block keeps a hint of the background under it.
    let cursor_col = crate::render::tokens::lerp_color(bg, cursor_col, cursor_alpha.min(0.9));
    let cursor_at = if cursor_alpha > 0.02
        && fr.cursor.visible
        && fr.cursor.x < fr.cols
        && fr.cursor.y < fr.rows
    {
        Some((fr.cursor.x, fr.cursor.y))
    } else {
        None
    };
    // Glass is uniform: the selection and explicit ANSI cell backgrounds
    // keep their color but share the pane fill alpha, so the whole pane
    // area sees through equally (ink stays opaque). bg_runs applies the
    // alpha itself; pass the OPAQUE selection color.
    let sel_opaque = to_c32(pal.selection_background);
    let font = FontId::monospace(font_size);
    let wide_font = FontId::monospace(cell.wide_size);
    // Cell backgrounds as MERGED maximal rectangles: epaint feathers
    // every rect edge, so one rect per cell leaves a lattice of faint
    // seams in same-color regions; full-width/full-height runs also
    // bleed into the pane's right/bottom remainder (the grid is
    // floor(pane/cell) cells wide and tall).
    for run in bg_runs::bg_runs(fr, default_fg, sel_opaque, fill_alpha) {
        painter.rect_filled(
            bg_runs::run_rect(rect, &run, cell, fr.cols, fr.rows),
            0.0,
            run.color,
        );
    }
    for (y, row) in fr.cells.iter().enumerate() {
        let mut skip_tail = false;
        for (x, cd) in row.iter().enumerate() {
            if skip_tail {
                // The tail of a wide pair: no glyph of its own (its
                // background, if any, is covered by the merged runs).
                skip_tail = false;
                continue;
            }
            let (ux, uy) = (x as u16, y as u16);
            if ux >= fr.cols || uy >= fr.rows {
                continue;
            }
            let span: u16 = if cd.wide { 2 } else { 1 };
            let r = cell_rect(rect, ux, uy, cell, span);
            let fg = cell_colors(cd, default_fg, bg).0;
            let is_cursor = cursor_at == Some((ux, uy));
            if is_cursor {
                painter.rect_filled(r, 2.0, cursor_col);
            }
            if !cd.text.is_empty() {
                // Cursor-block glyph ink: the cell's own effective bg at
                // full alpha (contrast on colored cells), else the pane
                // bg color - a semi-transparent fill would hide it on
                // glass.
                let glyph = if is_cursor {
                    bg_runs::cursor_ink(cd, default_fg, sel_opaque, bg_ink)
                } else {
                    fg
                };
                // Wide cells paint at the scaled size so the glyph fills
                // exactly the two-cell span; narrow cells are unchanged.
                let cell_font = if cd.wide { &wide_font } else { &font };
                painter.text(
                    Pos2::new(r.min.x, r.center().y),
                    Align2::LEFT_CENTER,
                    cd.text.as_str(),
                    cell_font.clone(),
                    glyph,
                );
            }
            if cd.underline {
                painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, fg));
            }
            if cd.wide {
                skip_tail = true;
            }
        }
    }
}

/// Dimmed placeholder for panes without a live session.
pub fn draw_dead(painter: &Painter, rect: Rect, bg: Color32, fg: Color32, msg: &str, font: f32) {
    painter.rect_filled(rect, 0.0, bg);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        msg,
        FontId::monospace(font),
        fg,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_conversions_roundtrip() {
        let lt = layout_tree::Rect {
            x: 1.0,
            y: 2.0,
            w: 30.0,
            h: 40.0,
        };
        assert_eq!(lt_rect(egui_rect(lt)), lt);
        let e = Rect::from_min_size(Pos2::new(5.0, 6.0), Vec2::new(7.0, 8.0));
        assert_eq!(egui_rect(lt_rect(e)), e);
    }

    #[test]
    fn merged_bg_paints_one_full_width_rect() {
        // Headless paint (same pattern as the preedit tests): a 5x3
        // frame of uniform red-bg cells must produce EXACTLY ONE red
        // rect shape - not 15 - and its right/bottom sides must bleed
        // to the pane edge (the grid covers only 5*9=45pt of the 49pt
        // pane and 3*18=54pt of the 60pt pane).
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        let ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("grid-bg-test"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
        );
        let row = || {
            (0..5)
                .map(|_| vt_pane::CellData {
                    text: " ".into(),
                    bg: Some(vt_pane::term::Color {
                        r: 200,
                        g: 20,
                        b: 20,
                    }),
                    ..vt_pane::CellData::default()
                })
                .collect::<Vec<_>>()
        };
        let fr = VtFrame {
            cols: 5,
            rows: 3,
            cells: vec![row(), row(), row()],
            ..Default::default()
        };
        let pane = Rect::from_min_size(Pos2::new(3.0, 5.0), Vec2::new(49.0, 60.0));
        draw_frame(
            ui.painter(),
            pane,
            &DrawArgs {
                fr: &fr,
                pal: &theme::builtin::dracula(),
                cell: CellSize {
                    w: 9.0,
                    h: 18.0,
                    wide_size: 15.0,
                    w_px: 9,
                    h_px: 18,
                },
                font_size: 15.0,
                cursor_alpha: 0.0,
                bg: Color32::from_rgb(40, 42, 54),
                fill_alpha: 0.5,
            },
        );
        let mut out = ctx.end_pass();
        let red = with_opacity(Color32::from_rgb(200, 20, 20), 0.5);
        let reds: Vec<&egui::Shape> = out
            .shapes
            .iter()
            .filter(|cs| matches!(&cs.shape, egui::Shape::Rect(r) if r.fill == red))
            .map(|cs| &cs.shape)
            .collect();
        assert_eq!(reds.len(), 1, "one merged run, not one rect per cell");
        let egui::Shape::Rect(r) = reds[0] else {
            unreachable!("filtered to rects above");
        };
        assert_eq!(r.rect.left(), pane.left());
        assert_eq!(
            r.rect.right(),
            pane.right(),
            "full-width runs bleed into the pane remainder"
        );
        assert_ne!(r.rect.width(), 5.0 * 9.0, "must not stop at the grid edge");
        assert_eq!(
            r.rect.bottom(),
            pane.bottom(),
            "full-height runs bleed into the pane bottom remainder"
        );
        assert_ne!(
            r.rect.height(),
            3.0 * 18.0,
            "must not stop at the grid bottom"
        );
        // Only the pane bg fill remains as the other solid rect.
        let pane_bg = with_opacity(Color32::from_rgb(40, 42, 54), 0.5);
        let bg_rects = out
            .shapes
            .iter()
            .filter(|cs| matches!(&cs.shape, egui::Shape::Rect(r) if r.fill == pane_bg))
            .count();
        assert_eq!(bg_rects, 1);
        out.textures_delta.clear();
    }

    #[test]
    fn wide_glyph_at_wide_size_spans_two_cells() {
        // Headless context: one begin/end pass initializes the font
        // atlas (fonts_mut panics before the first pass).
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        ctx.end_pass().textures_delta.clear();
        let ppp = ctx.pixels_per_point();

        // The invariant that matters (any size): a wide glyph painted at
        // wide_size fills exactly two narrow cells. With the Maple
        // primary font (adv(汉) == 2*adv(M)) the scale converges to ~1.0,
        // so wide_size lands on font_size within quantization noise.
        let probe = |size: f32| {
            let cell = measure_cells(&ctx, size);
            let advance = ctx.fonts_mut(|f| {
                f.layout_no_wrap(
                    "\u{6C49}".to_owned(),
                    FontId::monospace(cell.wide_size),
                    Color32::WHITE,
                )
                .rect
                .width()
            });
            (cell, advance)
        };

        // Default size: the Maple advance (0.6em) * 15 is exactly 9.0
        // device pixels, so the snapped pitch is integral with no
        // rounding loss (ppp == 1.0 in this headless context).
        let (cell, advance) = probe(crate::state::DEFAULT_FONT_SIZE);
        assert!(
            (cell.w * ppp).fract() < 1e-4 && (cell.h * ppp).fract() < 1e-4,
            "snapped pitch must be whole device pixels (w {}, h {})",
            cell.w * ppp,
            cell.h * ppp
        );
        assert!(
            (cell.w * ppp - 9.0).abs() < 1e-4,
            "Maple 15pt pitch should be exactly 9px, got {}",
            cell.w * ppp
        );
        assert!(
            (advance - 2.0 * cell.w).abs() < 0.05,
            "wide advance {advance} should fill two cells ({}pts)",
            2.0 * cell.w
        );
        assert!(
            (cell.wide_size - crate::state::DEFAULT_FONT_SIZE).abs() < 0.2,
            "wide scale should converge to ~1.0 (got {}; ~1.2 means the Noto fallback won the probe)",
            cell.wide_size
        );

        // 14pt is the snapping case: raw Maple advance 8.4pt quantizes
        // down to a whole 8px pitch, wide cells to 16px, and the scaled
        // wide font still fills the two-cell span exactly.
        let (cell, advance) = probe(14.0);
        assert_eq!(cell.w * ppp, 8.0);
        assert_eq!(cell.w_px, 8);
        assert!(
            (advance - 2.0 * cell.w).abs() < 0.05,
            "wide advance {advance} should fill two snapped cells ({}pts)",
            2.0 * cell.w
        );
    }
}
