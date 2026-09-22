//! IME preedit overlay: pure painting, no state.

use egui::{Align2, Color32, Context, FontId, Painter, Pos2, Rect, Stroke, Vec2};

use crate::state::CellSize;

/// Paint the live IME preedit (composition) string at the focused pane's
/// cursor cell with the SAME conventions as committed grid text, so the
/// composition previews exactly how it will land: one glyph per cell on
/// the snapped pitch (advance-2 glyphs get the wide font and a two-cell
/// span), vertically centered like every grid glyph. The composition
/// underline is egui's own `visuals.ime_composition.active_underline_stroke`
/// (the exact stroke every other input surface in this app, rename
/// editors and text fields, uses) instead of a one-off accent pill.
pub fn paint(
    painter: &Painter,
    at: Rect,
    text: &str,
    cell: CellSize,
    font_size: f32,
    fg: Color32,
    underline: Stroke,
) {
    let narrow = FontId::monospace(font_size);
    let wide = FontId::monospace(cell.wide_size);
    let mut x = at.left();
    for ch in text.chars() {
        let (span, font) = if char_span(painter.ctx(), ch, &narrow, cell) == 2 {
            (2u16, &wide)
        } else {
            (1u16, &narrow)
        };
        let r = Rect::from_min_size(
            Pos2::new(x, at.top()),
            Vec2::new(cell.w * f32::from(span), at.height()),
        );
        painter.text(
            Pos2::new(r.min.x, r.center().y),
            Align2::LEFT_CENTER,
            ch.to_string(),
            font.clone(),
            fg,
        );
        x = r.right();
    }
    // Underline spans the whole composition (no arbitrary cap); the pane
    // clip keeps a long preedit from bleeding past the pane rect.
    if x > at.left() {
        painter.line_segment([at.left_bottom(), Pos2::new(x, at.bottom())], underline);
    }
}

/// Advance-based wide classification through the same family chain the
/// grid measures with: a glyph advancing ~2 cells is wide (matches how
/// committed CJK lands on the grid).
fn char_span(ctx: &Context, ch: char, narrow: &FontId, cell: CellSize) -> u16 {
    let advance = ctx.fonts_mut(|f| f.glyph_width(narrow, ch));
    if advance > cell.w * 1.5 {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Rect;

    fn paint_in_ui(at: Rect, text: &str, cell: CellSize, underline: Stroke) {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        // 0.36 panels need a parent Ui; build one directly on the root
        // layer instead (no container needed).
        let ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("preedit-test"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
        );
        paint(
            ui.painter(),
            at,
            text,
            cell,
            15.0,
            Color32::WHITE,
            underline,
        );
        // Nothing consumes this context: drop the unapplied font texture
        // delta like the other headless egui tests do.
        ctx.end_pass().textures_delta.clear();
    }

    fn cell() -> CellSize {
        CellSize {
            w: 9.0,
            h: 18.0,
            wide_size: 15.0,
            w_px: 9,
            h_px: 18,
        }
    }

    #[test]
    fn paint_runs_for_narrow_wide_and_empty() {
        let at = Rect::from_min_size(egui::pos2(4.0, 8.0), egui::vec2(9.0, 18.0));
        paint_in_ui(at, "a汉b", cell(), Stroke::new(2.0, Color32::LIGHT_BLUE));
        paint_in_ui(at, "", cell(), Stroke::new(2.0, Color32::LIGHT_BLUE));
    }

    #[test]
    fn span_classification_narrow_vs_wide() {
        let ctx = egui::Context::default();
        // Install the app font chain: the default egui fonts have no Han
        // glyph, so '汉' would resolve to nothing and classify narrow.
        crate::ui::fonts::install(&ctx);
        ctx.begin_pass(egui::RawInput::default());
        let font = FontId::monospace(15.0);
        assert_eq!(char_span(&ctx, 'M', &font, cell()), 1);
        assert_eq!(char_span(&ctx, '汉', &font, cell()), 2);
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn egui_composition_stroke_is_a_visible_underline() {
        // The stroke the call site passes is egui's own composition
        // stroke; assert the dark-theme default is a real underline so a
        // style regression cannot silently blank the preedit.
        let ctx = egui::Context::default();
        let stroke = ctx
            .style_of(egui::Theme::Dark)
            .visuals
            .ime_composition
            .active_underline_stroke;
        assert!(stroke.width >= 1.0, "a visible underline, not a hairline");
        assert!(stroke.color.a() > 0);
    }
}
