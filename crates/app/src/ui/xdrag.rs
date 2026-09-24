//! Cross-window tab-chip drag plumbing: screen-geometry publishing,
//! pointer ground truth for the live drag, and the target window's drop
//! highlight. The state surgery on drop lives in
//! [`crate::actions::winops`].

use egui::{Pos2, Rect, Ui};
use layout_tree::PaneId;
use theme::Palette;

use crate::input;
use crate::render::colors::{self, to_c32};
use crate::state::{Data, XTabDrag};

/// Record THIS window's strip/outer rects in screen points. Runs in
/// every window's pass: inside a viewport pass `i.viewport()` is THAT
/// viewport, so `outer_rect` is this window's monitor-space rect. The
/// root pass additionally prunes ids of windows that no longer exist
/// (and any drag whose source window vanished mid-gesture).
pub fn publish(ui: &Ui, d: &mut Data, strip: Rect) {
    let Some(win_id) = d.st.win().map(|w| w.id) else {
        return;
    };
    match ui.ctx().input(|i| i.viewport().outer_rect) {
        Some(outer) => {
            d.ui.screens
                .strip
                .insert(win_id, strip.translate(outer.min.to_vec2()));
            d.ui.screens.outer.insert(win_id, outer);
        }
        // Wayland cannot report window positions: this window contributes
        // no hit-test data (other windows' entries stay usable).
        None => {
            d.ui.screens.strip.remove(&win_id);
            d.ui.screens.outer.remove(&win_id);
        }
    }
    if d.st.windows.first().map(|w| w.id) == Some(win_id) {
        let live: Vec<u64> = d.st.windows.iter().map(|w| w.id).collect();
        d.ui.screens.outer.retain(|k, _| live.contains(k));
        d.ui.screens.strip.retain(|k, _| live.contains(k));
    }
    if let Some(x) = d.ui.xdrag.as_ref() {
        if !d.st.windows.iter().any(|w| w.id == x.source) {
            d.ui.xdrag = None;
            d.ui.pointer_screen = None;
        }
    }
}

/// Refresh the cross-window drag while THIS (source) window's chip drag
/// is live. Returns `(over, primary_down)`: the hovered target window
/// (if any) and the POLLED primary-button truth (None where the X11
/// poll is unavailable - Wayland/tests - so the event path decides the
/// release). Updates `ui.pointer_screen` / `ui.xdrag`.
pub fn step(ui: &Ui, d: &mut Data, anchor: PaneId, source: u64) -> (Option<u64>, Option<bool>) {
    let order: Vec<u64> = d.st.windows.iter().map(|w| w.id).collect();
    // X11 pointer poll: the server answers position + button mask with
    // no event delivery involved, so the pointer stays tracked while it
    // hovers ANOTHER window (same rationale as the edge-resize poll).
    let polled = input::pointer_poll::poll().map(|p| {
        let s = ui.ctx().pixels_per_point().max(1.0);
        (egui::pos2(p.x / s, p.y / s), p.primary_down)
    });
    let pos = polled
        .map(|(p, _)| Some(p))
        .unwrap_or_else(|| local_screen(ui));
    d.ui.pointer_screen = pos;
    let over =
        pos.and_then(|p| crate::actions::winops::strip_hit(&d.ui.screens, source, p, &order));
    d.ui.xdrag = Some(XTabDrag {
        anchor,
        source,
        over,
    });
    (over, polled.map(|(_, down)| down))
}

/// Pointer in screen points from the event path alone: the local
/// latest_pos translated by this viewport's origin.
fn local_screen(ui: &Ui) -> Option<Pos2> {
    ui.input(|i| {
        let local = i.pointer.latest_pos()?;
        let origin = i.viewport().outer_rect.or(i.viewport().inner_rect)?;
        Some(origin.min + local.to_vec2())
    })
}

/// Pointer in THIS window's local points (for the target-side highlight).
pub fn local_pointer(ui: &Ui, screen: Option<Pos2>) -> Option<Pos2> {
    let screen = screen?;
    let origin = ui
        .ctx()
        .input(|i| i.viewport().outer_rect.or(i.viewport().inner_rect))?;
    Some(screen - origin.min.to_vec2())
}

/// Target-side highlight, part 1: soft accent tint over the strip.
/// Painted BEFORE the chips are laid out, so it reads as a tinted bar
/// under the chip ink (dropzone-style solid mix, no alpha).
pub fn paint_strip_tint(ui: &Ui, pal: &Palette, strip: Rect) {
    let col = to_c32(colors::mix(
        colors::chrome_bg(pal),
        pal.block_highlight,
        0.22,
    ));
    ui.painter().rect_filled(strip, 2.0, col);
}

/// Target-side highlight, part 2: 2px accent insertion caret at the gap
/// beside the chip whose center is nearest the pointer (needs the chip
/// rects allocated by the row, so it runs after the chip loop).
pub fn paint_caret(
    ui: &Ui,
    pal: &Palette,
    strip: Rect,
    chips: &[(Rect, usize)],
    pointer_local: Option<Pos2>,
    gap: f32,
) {
    let Some(px) = pointer_local.map(|p| p.x) else {
        return;
    };
    let Some(x) = caret_x(chips, px, &strip, gap) else {
        return;
    };
    ui.painter().rect_filled(
        Rect::from_min_max(
            egui::pos2(x - 1.0, strip.top() + 2.0),
            egui::pos2(x + 1.0, strip.bottom() - 2.0),
        ),
        1.0,
        to_c32(pal.block_highlight),
    );
}

/// Caret x (pure): beside the chip whose center is nearest `px`
/// (after it when the pointer sits right of center, before otherwise),
/// clamped into the strip.
pub fn caret_x(chips: &[(Rect, usize)], px: f32, strip: &Rect, gap: f32) -> Option<f32> {
    let (r, _) = chips.iter().copied().min_by(|a, b| {
        (a.0.center().x - px)
            .abs()
            .total_cmp(&(b.0.center().x - px).abs())
    })?;
    let x = if px > r.center().x {
        r.right() + gap * 0.5
    } else {
        r.left() - gap * 0.5
    };
    Some(x.clamp(strip.left(), strip.right()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_x_snaps_to_nearest_chip_gap() {
        let strip = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(300.0, 30.0));
        let chips = [
            (
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(50.0, 30.0)),
                0,
            ),
            (
                Rect::from_min_max(egui::pos2(55.0, 0.0), egui::pos2(105.0, 30.0)),
                1,
            ),
        ];
        // Right of chip 1's center: caret in the gap after it.
        assert_eq!(caret_x(&chips, 81.0, &strip, 5.0), Some(107.5));
        // Left of chip 0's center: before the first chip (clamped at 0).
        assert_eq!(caret_x(&chips, 10.0, &strip, 5.0), Some(0.0));
        // Between centers: nearest wins (chip 1 at 80 -> gap BEFORE it,
        // chip 0 at 25 must not win on distance).
        assert_eq!(caret_x(&chips, 79.0, &strip, 5.0), Some(52.5));
        // Dead center of a chip counts as the left side of that chip.
        assert_eq!(caret_x(&chips, 80.0, &strip, 5.0), Some(52.5));
        // No chips: no caret.
        assert_eq!(caret_x(&[], 10.0, &strip, 5.0), None);
    }
}
