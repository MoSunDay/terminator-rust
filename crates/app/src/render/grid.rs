//! Terminal Frame -> egui painter grid drawing.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use theme::{mix, Palette, Rgb};
use vt_pane::Frame as VtFrame;

use crate::render::colors::{cell_colors, effective_bg, from_c32, to_c32, vt_rgb, with_opacity};
use crate::state::{CellSize, PaneMeta};

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
    // A wide cell must span exactly two narrow cells: derive the paint
    // size by scaling the probe advance to 2*w. Degenerate measurements
    // (no CJK glyph found, zero advance) fall back to a sane 1.2x.
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

/// Bundled arguments of [`draw_frame`] (keeps the call sites readable).
pub struct DrawArgs<'a> {
    pub fr: &'a VtFrame,
    pub pal: &'a Palette,
    pub meta: &'a PaneMeta,
    pub cell: CellSize,
    pub font_size: f32,
    pub cursor_on: bool,
    /// Window opacity applied to the pane base fill only.
    pub opacity: f32,
}

/// Fade explicit cell backgrounds toward the pane background by the
/// pane's transparency, so ANSI-colored cells stay see-through.
fn blend_cell_bg(cbg: Color32, pane_bg: Rgb, t: f32) -> Color32 {
    if t <= 0.0 {
        return cbg;
    }
    to_c32(mix(pane_bg, from_c32(cbg), 1.0 - t))
}

/// Draw one terminal snapshot into `rect`.
///
/// Background first (pane effective bg), then per-cell bg overrides, the
/// cursor block, text, and underlines. Wide cells span two columns and the
/// trailing empty cell after them is skipped (per row: a row-final wide
/// cell must not eat the next row's first cell).
pub fn draw_frame(painter: &Painter, rect: Rect, a: &DrawArgs<'_>) {
    let fr = a.fr;
    let pal = a.pal;
    let meta = a.meta;
    let cell = a.cell;
    let font_size = a.font_size;
    let cursor_on = a.cursor_on;
    let bg = with_opacity(effective_bg(pal, meta), a.opacity);
    painter.rect_filled(rect, 0.0, bg);
    let default_fg = to_c32(pal.foreground);
    let cursor_col = fr
        .cursor_color
        .map(vt_rgb)
        .map_or(to_c32(pal.cursor), to_c32);
    let cursor_at =
        if cursor_on && fr.cursor.visible && fr.cursor.x < fr.cols && fr.cursor.y < fr.rows {
            Some((fr.cursor.x, fr.cursor.y))
        } else {
            None
        };
    let t = meta.transparency.clamp(0.0, 1.0);
    let bg_rgb = from_c32(bg);
    let sel_bg = blend_cell_bg(to_c32(pal.selection_background), bg_rgb, t);
    let font = FontId::monospace(font_size);
    let wide_font = FontId::monospace(cell.wide_size);
    for (y, row) in fr.cells.iter().enumerate() {
        let mut skip_tail = false;
        for (x, cd) in row.iter().enumerate() {
            if skip_tail {
                skip_tail = false;
                // Still paint the tail cell's explicit background.
                let (_, cbg) = cell_colors(cd, default_fg, bg);
                let cbg = if cd.selected {
                    Some(sel_bg)
                } else {
                    cbg.map(|c| blend_cell_bg(c, bg_rgb, t))
                };
                if let Some(cbg) = cbg {
                    let r = cell_rect(rect, x as u16, y as u16, cell, 1);
                    painter.rect_filled(r, 0.0, cbg);
                }
                continue;
            }
            let (ux, uy) = (x as u16, y as u16);
            if ux >= fr.cols || uy >= fr.rows {
                continue;
            }
            let span: u16 = if cd.wide { 2 } else { 1 };
            let r = cell_rect(rect, ux, uy, cell, span);
            let (fg, cbg) = cell_colors(cd, default_fg, bg);
            let cbg = if cd.selected {
                Some(sel_bg)
            } else {
                cbg.map(|c| blend_cell_bg(c, bg_rgb, t))
            };
            if let Some(cbg) = cbg {
                painter.rect_filled(r, 0.0, cbg);
            }
            let is_cursor = cursor_at == Some((ux, uy));
            if is_cursor {
                painter.rect_filled(r, 0.0, cursor_col);
            }
            if !cd.text.is_empty() {
                let glyph = if is_cursor { bg } else { fg };
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
pub fn draw_dead(painter: &Painter, rect: Rect, bg: Color32, fg: Color32, msg: &str) {
    painter.rect_filled(rect, 0.0, bg);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        msg,
        FontId::monospace(13.0),
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
    fn wide_glyph_at_wide_size_spans_two_cells() {
        // Headless context: one begin/end pass initializes the font
        // atlas (fonts_mut panics before the first pass).
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        ctx.end_pass().textures_delta.clear();
        let cell = measure_cells(&ctx, 14.0);
        let wide_font = FontId::monospace(cell.wide_size);
        let advance = ctx.fonts_mut(|f| {
            f.layout_no_wrap("\u{6C49}".to_owned(), wide_font, Color32::WHITE)
                .rect
                .width()
        });
        // The invariant that matters: a wide glyph painted at wide_size
        // fills exactly two narrow cells (embedded font scale ~1.2).
        assert!(
            (advance - 2.0 * cell.w).abs() < 0.05,
            "wide advance {advance} should fill two cells ({}pts)",
            2.0 * cell.w
        );
        assert!(
            cell.wide_size > 14.0,
            "wide cells must paint larger than narrow ones: {}",
            cell.wide_size
        );
    }
}
