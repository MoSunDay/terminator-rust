//! Terminal Frame -> egui painter grid drawing.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};
use theme::Palette;
use vt_pane::Frame as VtFrame;

use crate::render::colors::{cell_colors, effective_bg, to_c32, vt_rgb};
use crate::state::{CellSize, PaneMeta};

/// Measure the monospace cell metrics from egui fonts.
pub fn measure_cells(ctx: &egui::Context, font_size: f32) -> CellSize {
    let font = FontId::monospace(font_size);
    let ppp = ctx.pixels_per_point();
    let (w, h) = ctx.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let w = w.max(4.0);
    let h = h.max(6.0);
    CellSize {
        w,
        h,
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
}

/// Draw one terminal snapshot into `rect`.
///
/// Background first (pane effective bg), then per-cell bg overrides, the
/// cursor block, text, and underlines. Wide cells span two columns and the
/// trailing empty cell after them is skipped.
pub fn draw_frame(painter: &Painter, rect: Rect, a: &DrawArgs<'_>) {
    let fr = a.fr;
    let pal = a.pal;
    let meta = a.meta;
    let cell = a.cell;
    let font_size = a.font_size;
    let cursor_on = a.cursor_on;
    let bg = effective_bg(pal, meta);
    painter.rect_filled(rect, 0.0, bg);
    let default_fg = to_c32(pal.foreground);
    let cursor_col = fr.cursor_color.map(vt_rgb).map_or(to_c32(pal.cursor), to_c32);
    let cursor_at = if cursor_on && fr.cursor.visible && fr.cursor.x < fr.cols && fr.cursor.y < fr.rows
    {
        Some((fr.cursor.x, fr.cursor.y))
    } else {
        None
    };
    let font = FontId::monospace(font_size);
    let mut skip_tail = false;
    for (y, row) in fr.cells.iter().enumerate() {
        for (x, cd) in row.iter().enumerate() {
            if skip_tail {
                skip_tail = false;
                continue;
            }
            let (ux, uy) = (x as u16, y as u16);
            if ux >= fr.cols || uy >= fr.rows {
                continue;
            }
            let span: u16 = if cd.wide { 2 } else { 1 };
            let r = cell_rect(rect, ux, uy, cell, span);
            let (fg, cbg) = cell_colors(cd, default_fg, bg);
            if let Some(cbg) = cbg {
                painter.rect_filled(r, 0.0, cbg);
            }
            let is_cursor = cursor_at == Some((ux, uy));
            if is_cursor {
                painter.rect_filled(r, 0.0, cursor_col);
            }
            if !cd.text.is_empty() && !is_cursor {
                painter.text(
                    Pos2::new(r.min.x, r.center().y),
                    Align2::LEFT_CENTER,
                    cd.text.as_str(),
                    font.clone(),
                    fg,
                );
            }
            if cd.underline {
                painter.line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1.0, fg),
                );
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
        let lt = layout_tree::Rect { x: 1.0, y: 2.0, w: 30.0, h: 40.0 };
        assert_eq!(lt_rect(egui_rect(lt)), lt);
        let e = Rect::from_min_size(Pos2::new(5.0, 6.0), Vec2::new(7.0, 8.0));
        assert_eq!(egui_rect(lt_rect(e)), e);
    }
}
