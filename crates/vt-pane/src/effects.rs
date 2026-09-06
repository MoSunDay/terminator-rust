//! VT effect callbacks: answer program queries that gate first render.
//!
//! Programs such as zellij probe the terminal (device attributes, pixel
//! size, color scheme) before drawing anything; without responses they
//! stall on their loading screen.

use std::sync::{Arc, Mutex};

use anyhow::Result;
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

/// Install the query-response effects on a freshly created terminal.
///
/// `dark` selects the color-scheme answer (CSI ? 996 n) reported to
/// programs; wire it to the active theme's background luminance.
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

    // Capture the Copy bool (not the enum) so the closure stays trivially
    // 'static and independent of ColorScheme's derives.
    term.on_color_scheme(move |_| {
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
