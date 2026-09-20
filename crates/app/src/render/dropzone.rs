//! Drop-zone overlay for the Ctrl+drag pane move: soft accent tint
//! ring-fencing where the pane will land.

use egui::{CornerRadius, Painter, Rect, Stroke, StrokeKind};
use theme::Palette;

use crate::render::colors::{self, to_c32};
use crate::render::tokens;

/// Paint the drag overlay: `source` is the dragged pane, `target` the
/// hovered drop pane, `preview` the rect the dragged pane would occupy.
/// Solid ladder blends (no alpha) so pixels stay deterministic. `source`
/// is None while a cross-tab drag pulls the pane from a tab that is not
/// on screen.
pub fn paint(painter: &Painter, pal: &Palette, source: Option<Rect>, target: Rect, preview: Rect) {
    // Dragged pane: a quiet seam so the source card stays readable.
    if let Some(source) = source {
        painter.rect_stroke(
            source,
            CornerRadius::same(tokens::R_SM),
            Stroke::new(
                1.0,
                to_c32(colors::mix(pal.background, pal.foreground, 0.35)),
            ),
            StrokeKind::Inside,
        );
    }
    // Drop target: accent ring.
    painter.rect_stroke(
        target,
        CornerRadius::same(tokens::R_SM),
        Stroke::new(
            1.5,
            to_c32(colors::mix(pal.background, pal.block_highlight, 0.55)),
        ),
        StrokeKind::Inside,
    );
    // Landing preview: soft accent tint + stronger ring.
    let radius = CornerRadius::same(tokens::R_MD);
    painter.rect_filled(
        preview,
        radius,
        to_c32(colors::mix(pal.background, pal.block_highlight, 0.22)),
    );
    painter.rect_stroke(
        preview,
        radius,
        Stroke::new(
            2.0,
            to_c32(colors::mix(pal.background, pal.block_highlight, 0.85)),
        ),
        StrokeKind::Inside,
    );
}
