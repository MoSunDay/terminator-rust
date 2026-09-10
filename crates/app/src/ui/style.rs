//! Theme-derived egui style, applied once per theme change.

use egui::{vec2, Context, CornerRadius, Stroke, Style, Visuals};

use crate::render::colors::{self, palette_of};
use crate::state::UiState;

/// Install a dark egui style derived from the active theme. Idempotent per
/// theme name: `UiState.styled_theme` tracks the last applied one.
pub fn sync(ctx: &Context, theme_name: &str, uist: &mut UiState) {
    if uist.styled_theme.as_deref() == Some(theme_name) {
        return;
    }
    uist.styled_theme = Some(theme_name.to_string());
    let pal = palette_of(theme_name);
    let chrome = colors::to_c32(colors::chrome_bg(&pal));
    let hover = colors::to_c32(colors::chrome_hover(&pal));
    let accent = colors::to_c32(pal.block_highlight);
    let text = colors::to_c32(pal.foreground);
    let line = colors::to_c32(colors::hairline(&pal));

    let mut style = Style {
        visuals: Visuals::dark(),
        ..Default::default()
    };
    style.spacing.item_spacing = vec2(7.0, 5.0);
    style.spacing.button_padding = vec2(8.0, 4.0);
    // Slim scrollbars (fields verified present in egui 0.36 ScrollStyle).
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.bar_outer_margin = 2.0;
    style.visuals.panel_fill = chrome;
    style.visuals.window_fill = chrome;
    style.visuals.faint_bg_color = hover;
    style.visuals.extreme_bg_color = colors::to_c32(pal.background);
    style.visuals.hyperlink_color = accent;
    style.visuals.window_stroke = Stroke::new(1.0, line);
    style.visuals.selection.bg_fill = accent.gamma_multiply(0.25);
    style.visuals.selection.stroke = Stroke::NONE;

    // egui Separators paint with the noninteractive bg_stroke.
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, line);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text.gamma_multiply(0.6));
    style.visuals.widgets.inactive.bg_fill = hover;
    style.visuals.widgets.inactive.weak_bg_fill = hover;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(4);
    style.visuals.widgets.hovered.weak_bg_fill = colors::to_c32(colors::divider(&pal));
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.6));
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(4);
    style.visuals.widgets.active.weak_bg_fill = accent.gamma_multiply(0.8);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, text);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(4);
    style.visuals.widgets.open.weak_bg_fill = hover;
    style.visuals.widgets.open.corner_radius = CornerRadius::same(4);
    // 0.36 splits the style store by light/dark; ours is a dark style.
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}
