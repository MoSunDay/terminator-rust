//! Design tokens: radius scale, elevation shadows, motion constants.
//! Pure functions and consts only - stateless by convention.

use std::sync::OnceLock;

use egui::{Color32, Context, Id, Shadow};

/// Radius scale (points): small controls, chips/cards, floating layers.
pub const R_SM: u8 = 4;
pub const R_MD: u8 = 6;
pub const R_LG: u8 = 10;

/// Hover fade duration (seconds).
pub const FADE_S: f32 = 0.12;
/// Cursor blink period (seconds), matching the legacy duty cycle.
pub const CURSOR_PERIOD_S: f32 = 2.0;

/// Kinds of floating layers that carry an elevation shadow.
#[derive(Clone, Copy)]
pub enum Layer {
    Window,
    Popup,
}

/// Soft elevation shadow for a floating layer (windows, popups, menus).
pub fn shadow(layer: Layer) -> Shadow {
    let (blur, alpha) = match layer {
        Layer::Window => (14, 96),
        Layer::Popup => (10, 80),
    };
    Shadow {
        offset: [0, 4],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    }
}

/// Motion kill-switch: TERMINATOR_NO_MOTION=1 pins every animation to its
/// end state (e2e pixel determinism). Read once per process.
pub fn motion_enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    !*OFF.get_or_init(|| {
        std::env::var_os("TERMINATOR_NO_MOTION").as_deref() == Some(std::ffi::OsStr::new("1"))
    })
}

/// Animated hover factor (0 = idle, 1 = hovered) with a short fade;
/// instant when motion is disabled.
pub fn hover_t(ctx: &Context, id: Id, hovered: bool) -> f32 {
    if !motion_enabled() {
        return f32::from(hovered);
    }
    ctx.animate_value_with_time(id, f32::from(hovered), FADE_S)
}

/// Premultiplied-channel lerp between two colors (`t` clamped to 0..=1).
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + t * (f32::from(y) - f32::from(x))).round() as u8;
    Color32::from_rgba_premultiplied(
        ch(a.r(), b.r()),
        ch(a.g(), b.g()),
        ch(a.b(), b.b()),
        ch(a.a(), b.a()),
    )
}

/// Cursor visibility alpha at time `t` (seconds): a sine soft-blink, or
/// the legacy 1.2s-of-2s square duty cycle when motion is disabled.
pub fn cursor_alpha(t: f32) -> f32 {
    if !motion_enabled() {
        return if (t * 2.0) % 2.0 < 1.2 { 1.0 } else { 0.0 };
    }
    0.5 + 0.5 * (t * std::f32::consts::TAU / CURSOR_PERIOD_S + std::f32::consts::FRAC_PI_2).sin()
}
