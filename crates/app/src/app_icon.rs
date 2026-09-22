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

use std::sync::{Arc, OnceLock};

/// The approved 256x256 logo raster, embedded at build time.
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

    #[test]
    fn embedded_logo_decodes_to_256x256_rgba() {
        let icon = icon().expect("embedded logo must decode");
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn embedded_logo_matches_the_approved_design() {
        let icon = icon().expect("embedded logo must decode");
        // (0,0) sits outside the rounded tile -> fully transparent.
        assert_eq!(px(icon, 0, 0)[3], 0, "corner must be transparent");
        // Tab bar (purple #bd93f9), the split's left leg (pink #ff79c6),
        // right leg (cyan #8be9fd) and the tile background (#282a2f-ish).
        assert_eq!(px(icon, 128, 66), [189, 147, 249, 255], "purple tab bar");
        assert_eq!(px(icon, 83, 170), [255, 121, 198, 255], "pink left leg");
        assert_eq!(px(icon, 173, 170), [139, 233, 253, 255], "cyan right leg");
        assert_eq!(px(icon, 128, 230), [35, 36, 47, 255], "tile background");
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
