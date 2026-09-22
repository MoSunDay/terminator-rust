//! Chrome metric derivation: tab chips, pane headers and the pinned
//! right-edge controls all scale with the terminal font size, so the
//! chrome grows with the terminal text. Every base value below is the
//! historical hard-coded constant; at the default font size (scale 1.0)
//! each metric is numerically identical to that constant (x * 1.0f32 is
//! exact), which the e2e pixel gates depend on.

use crate::state::DEFAULT_FONT_SIZE;

/// Chrome scale: 1.0 at the default terminal font size.
pub fn scale(font_size: f32) -> f32 {
    font_size / DEFAULT_FONT_SIZE
}

/// Scaled chrome metrics for one frame (egui points). Every value is the
/// historical hard-coded constant times the scale, so default-size
/// rendering is unchanged.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// The scale itself (1.0 at the default font size).
    pub s: f32,
    // --- tab chips ---
    /// Chip pill height (base 28).
    pub chip_h: f32,
    /// Chip horizontal text padding (base 10).
    pub chip_pad_x: f32,
    /// Minimum chip width (base 44).
    pub chip_min_w: f32,
    /// Gap between chips (base 5).
    pub chip_gap: f32,
    /// Width the rename TextEdit occupies in the chip flow (base 118).
    pub chip_edit_w: f32,
    /// Right strip a chip reserves for its close X (base 14).
    pub close_w: f32,
    /// Close X hit/glyph plate size (base 12).
    pub close_btn: f32,
    /// Active-chip accent underline height (base 2).
    pub accent_h: f32,
    /// Chip label font size: equal to the terminal font size.
    pub chip_font: f32,
    /// Top/bottom inset of the chip row (base 4).
    pub row_inset: f32,
    // --- pinned right chrome ---
    /// Control cell size (base 16).
    pub icon: f32,
    /// Control cell gutter (base 4).
    pub icon_gap: f32,
    /// Gap after the chip strip before the trailing group (base 8).
    pub group_pad: f32,
    /// Trailing (+/split) group width: 3 icon cells with gutters.
    pub group_w: f32,
    /// Chrome width reserved right of the chips (base 153): one chip
    /// gap + the trailing group + one gap + 4 edge cells.
    pub reserve: f32,
    /// Chip-strip wheel step per wheel line (base 48).
    pub wheel_step: f32,
    // --- pane header ---
    /// Pane header strip height (base 24).
    pub header_h: f32,
    /// Header close-button size (base 16).
    pub header_btn: f32,
    /// Header title font size (base 12).
    pub title_font: f32,
    /// Header badge font size (base 11).
    pub badge_font: f32,
    /// Header reject-note font size (base 11).
    pub note_font: f32,
    /// Dead-pane message font size (base 13).
    pub dead_font: f32,
}

/// Derive the frame's chrome metrics from the terminal font size.
pub fn metrics(font_size: f32) -> Metrics {
    let s = scale(font_size);
    let mut m = Metrics {
        s,
        chip_h: 28.0 * s,
        chip_pad_x: 10.0 * s,
        chip_min_w: 44.0 * s,
        chip_gap: 5.0 * s,
        chip_edit_w: 118.0 * s,
        close_w: 14.0 * s,
        close_btn: 12.0 * s,
        accent_h: 2.0 * s,
        chip_font: font_size,
        row_inset: 4.0 * s,
        icon: 16.0 * s,
        icon_gap: 4.0 * s,
        group_pad: 8.0 * s,
        group_w: 3.0 * (16.0 * s) + 2.0 * (4.0 * s),
        reserve: 0.0,
        wheel_step: 48.0 * s,
        header_h: 24.0 * s,
        header_btn: 16.0 * s,
        title_font: 12.0 * s,
        badge_font: 11.0 * s,
        note_font: 11.0 * s,
        dead_font: 13.0 * s,
    };
    // Chrome reserved right of the chips: one chip gap + the trailing
    // group + one gap + 4 edge cells (153 at the default font size).
    m.reserve = 5.0 * m.s + 4.0 * m.icon + 3.0 * m.icon_gap + m.group_pad + m.group_pad + m.group_w;
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_size_is_the_historical_constants() {
        let m = metrics(DEFAULT_FONT_SIZE);
        assert_eq!(m.s, 1.0);
        assert_eq!(m.chip_h, 28.0);
        assert_eq!(m.reserve, 153.0);
        assert_eq!(m.group_w, 56.0);
        assert_eq!(m.icon, 16.0);
        assert_eq!(m.header_h, 24.0);
        // The tab title renders at the terminal font size.
        assert_eq!(m.chip_font, DEFAULT_FONT_SIZE);
    }

    #[test]
    fn reserve_scales_linearly() {
        assert_eq!(
            metrics(30.0).reserve,
            2.0 * metrics(DEFAULT_FONT_SIZE).reserve
        );
    }

    #[test]
    fn chip_font_tracks_the_terminal_font() {
        assert_eq!(metrics(20.0).chip_font, 20.0);
        assert_eq!(metrics(9.0).chip_font, 9.0);
    }

    #[test]
    fn scale_is_monotonic() {
        assert_eq!(scale(DEFAULT_FONT_SIZE), 1.0);
        assert!((scale(10.0) - 10.0 / 15.0).abs() < 1e-6);
        assert_eq!(scale(24.0), 24.0 / 15.0);
        assert!(scale(10.0) < scale(DEFAULT_FONT_SIZE));
        assert!(scale(DEFAULT_FONT_SIZE) < scale(24.0));
    }
}
