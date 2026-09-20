//! IME preedit overlay: pure painting, no state.

use egui::{Color32, FontId, Painter, Rect, Vec2};

/// Paint the live IME preedit (composition) string at the focused pane's
/// cursor cell: the text over a 2px accent underline - the classic
/// terminal IME anchor look. Pure painting; callers own all decisions.
pub fn paint(
    painter: &Painter,
    at: Rect,
    text: &str,
    font_size: f32,
    fg: Color32,
    accent: Color32,
) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(font_size), fg);
    painter.galley(at.left_top(), galley.clone(), fg);
    // Cap the underline so a long composition cannot run off-window
    // (8 cells of headroom past the single-cell anchor rect).
    let w = galley.size().x.min(at.width() * 8.0).max(2.0);
    let underline =
        Rect::from_min_size(egui::pos2(at.left(), at.bottom() - 2.0), Vec2::new(w, 2.0));
    painter.rect_filled(underline, 1.0, accent);
}
