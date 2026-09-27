//! Paint styled terminal text without changing its fixed cell advance.
use egui::{Color32, FontId, Painter, Pos2};

pub fn draw(
    painter: &Painter,
    center_left: Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    italic: bool,
) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    for section in &mut job.sections {
        section.format.italics = italic;
    }
    let galley = painter.layout_job(job);
    let pos = Pos2::new(center_left.x, center_left.y - galley.size().y / 2.0);
    painter.galley(pos, galley, color);
}
