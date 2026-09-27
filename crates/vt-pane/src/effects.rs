//! VT effect callbacks: answer program queries that gate first render.
//!
//! Programs such as zellij probe the terminal (device attributes, pixel
//! size, color scheme) before drawing anything; without responses they
//! stall on their loading screen.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use libghostty_vt::style::RgbColor;
use libghostty_vt::terminal::SizeReportSize;
use libghostty_vt::terminal::{
    ColorScheme, ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType,
    PrimaryDeviceAttributes, SecondaryDeviceAttributes, TertiaryDeviceAttributes,
};
use libghostty_vt::Terminal;

/// Shared current cell pixel size, kept in sync by `task::resize`.
pub type CellPx = Arc<Mutex<(u32, u32)>>;

/// Nominal cell size until the UI reports real geometry.
const NOMINAL_CELL_PX: (u32, u32) = (8, 16);

fn dark_background(color: RgbColor) -> bool {
    let brightness =
        0.2126 * f32::from(color.r) + 0.7152 * f32::from(color.g) + 0.0722 * f32::from(color.b);
    brightness < 127.5
}

/// Install the query-response effects on a freshly created terminal.
///
/// `dark` is the fallback until a background is installed; color-scheme
/// queries subsequently reflect the terminal's effective OSC/theme color.
pub fn install(term: &mut Terminal<'static, 'static>, cell_px: CellPx, dark: bool) -> Result<()> {
    term.on_device_attributes(|_| {
        Some(DeviceAttributes {
            primary: PrimaryDeviceAttributes::new(
                ConformanceLevel::VT220,
                &[DeviceAttributeFeature::SELECTIVE_ERASE],
            ),
            secondary: SecondaryDeviceAttributes {
                device_type: DeviceType::VT220,
                firmware_version: 0,
                rom_cartridge: 0,
            },
            tertiary: TertiaryDeviceAttributes::default(),
        })
    })?;

    term.on_size(move |t| {
        let (w, h) = *cell_px.lock().unwrap_or_else(|e| e.into_inner());
        let (cols, rows) = (t.cols().unwrap_or(80), t.rows().unwrap_or(24));
        Some(SizeReportSize {
            rows,
            columns: cols,
            cell_width: w,
            cell_height: h,
        })
    })?;

    term.on_color_scheme(move |term| {
        let dark = term
            .bg_color()
            .ok()
            .flatten()
            .map(dark_background)
            .unwrap_or(dark);
        Some(if dark {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        })
    })?;
    Ok(())
}

/// Fresh shared cell size for a new session.
pub fn new_cell_px() -> CellPx {
    Arc::new(Mutex::new(NOMINAL_CELL_PX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_scheme_query_tracks_effective_background() {
        let mut term = Terminal::new(20, 4).expect("terminal");
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&output);
        term.on_pty_write(move |_, bytes| sink.lock().unwrap().extend_from_slice(bytes))
            .expect("pty response");
        install(&mut term, new_cell_px(), true).expect("effects");
        term.set_default_bg_color(Some(RgbColor {
            r: 12,
            g: 24,
            b: 36,
        }))
        .expect("dark default");
        term.vt_write(b"\x1b[?996n");
        term.vt_write(b"\x1b]11;#f0f0f0\x07\x1b[?996n");
        term.vt_write(b"\x1b]111\x07\x1b[?996n");
        let response = output.lock().unwrap().clone();
        assert_eq!(response, b"\x1b[?997;1n\x1b[?997;2n\x1b[?997;1n");
    }
}
