//! GPU surface-size guard.
//!
//! wgpu aborts the whole process with a fatal validation error when a
//! window's PHYSICAL pixel size exceeds the adapter's max texture extent
//! (`max_texture_dimension_2d`; 8192 on llvmpipe software adapters - a 5K
//! window at 2.0 scale already crosses it). eframe configures the surface
//! inside the winit `Resized` handler BEFORE any app frame runs, so an
//! after-the-fact clamp is always too late: the only prevention is a
//! window-level max-inner-size constraint, which the WM honors for every
//! realistic resize path (user drag, maximize/fullscreen, our own
//! edge-resize `request_inner_size` calls).
//!
//! Launch applies the conservative WebGPU-baseline cap (see main.rs and
//! windows::builder_for); [`apply`] refines it each frame to the adapter's
//! real limit - relaxing the cap on GPUs that support 16K+ textures so
//! huge-desktop users are not capped at 8192.

use egui::{Context, Id, Vec2, ViewportCommand};

/// WebGPU-guaranteed floor for `max_texture_dimension_2d`; used at launch
/// (before the adapter reports its real limit) and as the fallback.
pub const FALLBACK_MAX_TEXTURE_SIDE: usize = 8192;

/// Scale factor sanitized for division: non-finite or tiny values fall
/// back to 1.0 so the cap can never blow up to infinity.
fn safe_ppm(pixels_per_point: f32) -> f32 {
    if pixels_per_point.is_finite() && pixels_per_point >= 0.05 {
        pixels_per_point
    } else {
        1.0
    }
}

/// Pure: the safe max-inner-size in logical points for one viewport.
pub fn safe_cap_points(max_texture_side: usize, pixels_per_point: f32) -> Vec2 {
    let side = (max_texture_side.max(64) as f32) / safe_ppm(pixels_per_point);
    Vec2::splat(side.max(64.0))
}

/// Pure: does a viewport of `inner_points` points at `ppp` already exceed
/// the adapter limit (a resize that raced the WM cap)?
pub fn exceeds(inner_points: Vec2, pixels_per_point: f32, max_texture_side: usize) -> bool {
    let limit = max_texture_side.max(64) as f32;
    let phys = inner_points * safe_ppm(pixels_per_point);
    phys.x > limit + 0.5 || phys.y > limit + 0.5
}

/// Per-viewport, per-frame guard: (re)applies the precise WM cap and
/// clamps an already-oversized window back inside it. Sends commands only
/// on change (deduplicated via egui memory), so idle frames cost nothing.
pub fn apply(ctx: &Context) {
    let (mts, ppp, inner) = ctx.input(|i| {
        (
            i.max_texture_side,
            i.pixels_per_point,
            i.viewport().inner_rect.map(|r| r.size()),
        )
    });
    let cap = safe_cap_points(mts, ppp);
    let id = Id::new("surface_guard_cap").with(ctx.viewport_id());
    let changed = ctx.data_mut(|d| {
        if d.get_temp::<Vec2>(id) == Some(cap) {
            false
        } else {
            d.insert_temp(id, cap);
            true
        }
    });
    if changed {
        ctx.send_viewport_cmd(ViewportCommand::MaxInnerSize(cap));
    }
    if let Some(size) = inner {
        if exceeds(size, ppp, mts) {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(
                size.x.min(cap.x),
                size.y.min(cap.y),
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_scales_with_ppp_and_never_blows_up() {
        assert_eq!(safe_cap_points(8192, 1.0), Vec2::splat(8192.0));
        assert_eq!(safe_cap_points(8192, 2.0), Vec2::splat(4096.0));
        assert_eq!(safe_cap_points(8192, 2.5), Vec2::splat(3276.8));
        // Non-finite / tiny scale factors fall back instead of exploding.
        assert_eq!(safe_cap_points(8192, f32::NAN), Vec2::splat(8192.0));
        assert_eq!(safe_cap_points(8192, 0.0), Vec2::splat(8192.0));
        // A pathological 0 limit stays a sane, renderable cap.
        assert_eq!(safe_cap_points(0, 1.0), Vec2::splat(64.0));
    }

    #[test]
    fn exceeds_only_beyond_the_physical_limit() {
        assert!(!exceeds(Vec2::new(8192.0, 8192.0), 1.0, 8192));
        assert!(exceeds(Vec2::new(8193.0, 100.0), 1.0, 8192));
        // 4096 logical points at 2.0 = 8192 physical: still inside.
        assert!(!exceeds(Vec2::new(4096.0, 4096.0), 2.0, 8192));
        assert!(exceeds(Vec2::new(4200.0, 100.0), 2.0, 8192));
        // NaN inner sizes never trigger a clamp.
        assert!(!exceeds(Vec2::NAN, 1.0, 8192));
    }
}
