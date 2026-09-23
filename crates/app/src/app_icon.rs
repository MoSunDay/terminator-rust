//! Application/window icon (the `assets/logo` "split T" mark).
//!
//! The logo is generated deterministically by `scripts/bin/gen-logo.py`
//! into `assets/logo/` (PNG rasters + svg/ico/icns). The 256px raster is
//! embedded into the binary at build time (`include_bytes!`) and decoded
//! ONCE (lazily, behind a `OnceLock`) into an [`egui::IconData`]; every
//! viewport builder is wrapped with [`with_icon`] so the icon lands on
//! the ROOT window AND on every secondary OS window: winit/eframe do not
//! inherit the icon into immediate (secondary) viewports, so each builder
//! carries its own handle on the one shared decoded raster. eframe also
//! applies the icon at runtime (native `AppTitleIconSetter`), which is
//! what publishes X11 `_NET_WM_ICON` and the macOS dock icon.
//!
//! Decoding uses `eframe::icon_data::from_png_bytes` (the `image` crate is
//! already a transitive dependency of eframe - no new dependency). If the
//! embedded bytes ever fail to decode the app still runs, just with the
//! toolkit default icon (`log::warn` + `None`).
//!
//! On macOS the embedded raster is the Apple icon-template variant
//! (`icon-256-mac.png`, artwork in the centered 824/1024 body); everywhere
//! else it is the full-bleed `icon-256.png`. See [`ICON_PNG`].

use std::sync::{Arc, OnceLock};

/// The approved 256x256 logo raster, embedded at build time.
///
/// macOS gets the Apple icon-template variant: eframe's `AppTitleIconSetter`
/// publishes our RGBA verbatim through `NSApp setApplicationIconImage:`
/// (winit's own `set_window_icon` is a no-op there) and AppKit scales the
/// WHOLE raster into the icon slot, so a full-bleed tile renders ~24%
/// larger than every other app's icon. `icon-256-mac.png` keeps the artwork
/// inside Apple's 824/1024 template body (a 25px transparent margin per
/// side at 256). Linux/Windows scale the icon themselves and want the
/// full-bleed art.
#[cfg(target_os = "macos")]
const ICON_PNG: &[u8] = include_bytes!("../../../assets/logo/icon-256-mac.png");
#[cfg(not(target_os = "macos"))]
const ICON_PNG: &[u8] = include_bytes!("../../../assets/logo/icon-256.png");

/// Decoded icon, shared across viewports (decoded at most once).
static ICON: OnceLock<Option<Arc<egui::IconData>>> = OnceLock::new();

/// Decode (once) and cache the embedded logo.
fn load() -> &'static Option<Arc<egui::IconData>> {
    ICON.get_or_init(|| match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(data) => Some(Arc::new(data)),
        Err(err) => {
            log::warn!("window icon: embedded logo failed to decode ({err}) - using the default");
            None
        }
    })
}

/// The decoded application icon, or `None` when the embedded PNG could not
/// be decoded (the app keeps working without a custom icon).
pub fn icon() -> Option<&'static egui::IconData> {
    load().as_deref()
}

/// Attach the application icon to `builder` when it is available; returns
/// the builder unchanged otherwise (never fails, never panics).
pub fn with_icon(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    match load() {
        Some(data) => builder.with_icon(Arc::clone(data)),
        None => builder,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RGBA sample of `icon` at (x, y) - pins the embedded bytes to the
    /// approved design.
    fn px(icon: &egui::IconData, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * icon.width + x) * 4) as usize;
        [
            icon.rgba[i],
            icon.rgba[i + 1],
            icon.rgba[i + 2],
            icon.rgba[i + 3],
        ]
    }

    /// Bounding box of the inked (alpha > 0) area as
    /// `(first x, first y, side length)` - the platform padding guard reads
    /// the Apple template margin (macOS) or its absence (everywhere else)
    /// straight off it, and [`design_px`] uses it to locate the artwork.
    fn ink_box(icon: &egui::IconData) -> (u32, u32, u32) {
        let mut acc: Option<(u32, u32, u32, u32)> = None;
        for y in 0..icon.height {
            for x in 0..icon.width {
                if px(icon, x, y)[3] == 0 {
                    continue;
                }
                acc = Some(match acc {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
        let (x0, y0, x1, _) = acc.expect("the embedded logo must carry ink");
        (x0, y0, x1 - x0 + 1)
    }

    /// Sample `icon` at a point of the generator's 1024u DESIGN grid, mapped
    /// into the inked body (rounded). Geometry-derived so one set of
    /// assertions is correct for the full-bleed raster AND for the macOS
    /// template raster, whose body is inset by the template margin.
    fn design_px(icon: &egui::IconData, dx: u32, dy: u32) -> [u8; 4] {
        let (x0, y0, len) = ink_box(icon);
        let at = |d: u32| ((d * len + 512) / 1024).min(len - 1);
        px(icon, x0 + at(dx), y0 + at(dy))
    }

    #[test]
    fn embedded_logo_decodes_to_256x256_rgba() {
        let icon = icon().expect("embedded logo must decode");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn embedded_logo_matches_the_approved_design() {
        let icon = icon().expect("embedded logo must decode");
        // (0,0) is transparent on every platform: outside the rounded tile
        // (full-bleed) or inside the Apple template margin (macOS).
        assert_eq!(px(icon, 0, 0)[3], 0, "corner must be transparent");
        // Sampled in DESIGN coords (the generator's 1024u grid) so the same
        // assertions hold for the full-bleed and the mac-template raster:
        // tab bar + stem (purple #bd93f9), the split's left leg (pink
        // #ff79c6), right leg (cyan #8be9fd), tile background (#282a2f-ish).
        assert_eq!(
            design_px(icon, 512, 262),
            [189, 147, 249, 255],
            "purple tab bar"
        );
        assert_eq!(
            design_px(icon, 512, 400),
            [189, 147, 249, 255],
            "purple stem"
        );
        assert_eq!(
            design_px(icon, 332, 680),
            [255, 121, 198, 255],
            "pink left leg"
        );
        assert_eq!(
            design_px(icon, 692, 680),
            [139, 233, 253, 255],
            "cyan right leg"
        );
        assert_eq!(
            design_px(icon, 512, 920),
            [35, 36, 47, 255],
            "tile background"
        );
    }

    /// The actual macOS bug guard: eframe publishes this raster verbatim via
    /// `setApplicationIconImage:`, so it MUST carry the Apple icon-template
    /// margin (body = 824/1024 of the canvas, centered -> 25px inset and a
    /// 206px body at 256) or the Dock icon renders ~24% too large.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_icon_carries_the_apple_template_margin() {
        let icon = icon().expect("embedded logo must decode");
        assert_eq!(
            ink_box(icon),
            (25, 25, 206),
            "macOS icon must keep the 824/1024 Apple template body"
        );
    }

    /// Linux/Windows scale the icon into their own slots, so their art stays
    /// FULL-BLEED - a margin there would render visibly smaller.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn full_bleed_icon_reaches_the_canvas_edge() {
        let icon = icon().expect("embedded logo must decode");
        assert_eq!(
            ink_box(icon),
            (0, 0, 256),
            "non-macOS icon must stay full-bleed"
        );
    }

    #[test]
    fn with_icon_attaches_the_logo_to_a_viewport_builder() {
        let builder = with_icon(egui::ViewportBuilder::default());
        let attached = builder.icon.expect("icon must be attached");
        assert_eq!((attached.width, attached.height), (256, 256));
    }

    #[test]
    fn icon_is_decoded_once_and_shared_by_every_viewport() {
        let a = with_icon(egui::ViewportBuilder::default())
            .icon
            .expect("icon must be attached");
        let b = with_icon(egui::ViewportBuilder::default())
            .icon
            .expect("icon must be attached");
        assert!(
            Arc::ptr_eq(&a, &b),
            "every viewport must share ONE decoded icon"
        );
        assert_eq!(
            icon().expect("icon() must be available").rgba.len(),
            a.rgba.len()
        );
    }
}
