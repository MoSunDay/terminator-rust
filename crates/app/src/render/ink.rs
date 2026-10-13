//! Paint styled terminal text without changing its fixed cell advance.
use egui::{Color32, Context, FontId, Pos2};

/// Shape one cell's glyph run: `None` when the galley is empty (mirrors
/// `Painter::galley`'s skip), else the galley shape vertically centered
/// on `center_left` (same geometry the direct-paint path used).
pub fn shape(
    ctx: &Context,
    center_left: Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    italic: bool,
) -> Option<egui::Shape> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    for section in &mut job.sections {
        section.format.italics = italic;
    }
    let galley = ctx.fonts_mut(|f| f.layout_job(job));
    if galley.is_empty() {
        return None;
    }
    let pos = Pos2::new(center_left.x, center_left.y - galley.size().y / 2.0);
    Some(egui::Shape::galley(pos, galley, color))
}
