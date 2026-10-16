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
/// Cursor blink period (seconds).
pub const CURSOR_PERIOD_S: f32 = 2.0;
/// Cursor blink on-time (seconds) within [`CURSOR_PERIOD_S`]: 1.2s on,
/// 0.8s off - the classic terminal duty cycle.
pub const CURSOR_ON_S: f32 = 1.2;

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

/// Cursor visibility alpha at time `t` (seconds): a square-wave blink
/// (1.2s on / 0.8s off per 2s), matching the blink-attribute text duty
/// cycle and mainstream terminals. Square edges keep the next blink
/// moment schedulable - `render::cadence` computes the exact wakeup
/// instead of re-rendering continuously (a soft sine never rests).
pub fn cursor_alpha(t: f32) -> f32 {
    // NO_MOTION pins the blink to its end state (visible): pixel-scraping
    // e2e runs and `cadence::next_deadline` (which skips the edge when
    // motion is off) then agree on a static cursor.
    if !motion_enabled() {
        return 1.0;
    }
    if t.rem_euclid(CURSOR_PERIOD_S) < CURSOR_ON_S {
        1.0
    } else {
        0.0
    }
}
