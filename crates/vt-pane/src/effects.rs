//! VT effect callbacks: answer program queries that gate first render.
//!
//! Programs such as zellij probe the terminal (device attributes, pixel
//! size, color scheme) before drawing anything; without responses they
//! stall on their loading screen.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
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

/// Shared "desktop notification seen" flag: any OSC 9 / OSC 777 desktop
/// notification parsed from the PTY stream sets it; the UI drains it once
/// per frame via `task::take_notice` to raise the pane's attention badge.
pub type NoticeFlag = Arc<AtomicBool>;

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
/// `notice` is marked whenever the program emits a desktop notification.
pub fn install(
    term: &mut Terminal<'static, 'static>,
    cell_px: CellPx,
    dark: bool,
    notice: NoticeFlag,
    copy_transport: Arc<AtomicU8>,
) -> Result<()> {
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

    // Any desktop notification (OSC 9 iTerm2-style, OSC 777 rxvt-style)
    // marks the pane for attention; title/body are irrelevant here - the
    // UI raises its notice badge, not an OS toast.
    term.on_desktop_notification(move |_term, notif| {
        if notif.title() == "terminator-rust" && notif.body() == "copy-transport-v1" {
            copy_transport.store(1, Ordering::Release);
        } else {
            notice.store(true, Ordering::Release);
        }
    })?;
    Ok(())
}

/// Fresh shared cell size for a new session.
pub fn new_cell_px() -> CellPx {
    Arc::new(Mutex::new(NOMINAL_CELL_PX))
}

/// Fresh notice flag for a new session.
pub fn new_notice() -> NoticeFlag {
    Arc::new(AtomicBool::new(false))
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
        install(
            &mut term,
            new_cell_px(),
            true,
            new_notice(),
            Arc::new(AtomicU8::new(0)),
        )
        .expect("effects");
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

    #[test]
    fn desktop_notification_osc_marks_the_notice_flag() {
        let mut term = Terminal::new(20, 4).expect("terminal");
        let notice = new_notice();
        install(
            &mut term,
            new_cell_px(),
            true,
            Arc::clone(&notice),
            Arc::new(AtomicU8::new(0)),
        )
        .expect("effects");
        // The exact bytes the remote keeper injects (OSC 9, BEL-terminated),
        // fed through the same vt_write call task::pump uses for PTY output.
        term.vt_write(b"\x1b]9;terminator-rust notice\x07");
        assert!(notice.load(Ordering::Acquire), "OSC 9 must mark the pane");
        // take_notice semantics: the swap yields true exactly once.
        assert!(notice.swap(false, Ordering::AcqRel));
        assert!(!notice.swap(false, Ordering::AcqRel));
        // A later notification re-marks the drained flag.
        term.vt_write(b"\x1b]9;terminator-rust notice\x07");
        assert!(notice.swap(false, Ordering::AcqRel));
        // Non-notification output leaves the flag at rest.
        term.vt_write(b"plain\r\n");
        assert!(!notice.swap(false, Ordering::AcqRel));
    }

    #[test]
    fn rxvt_osc777_notification_also_marks_the_notice_flag() {
        let mut term = Terminal::new(20, 4).expect("terminal");
        let notice = new_notice();
        install(
            &mut term,
            new_cell_px(),
            true,
            Arc::clone(&notice),
            Arc::new(AtomicU8::new(0)),
        )
        .expect("effects");
        term.vt_write(b"\x1b]777;notify;terminator-rust;notice\x07");
        assert!(notice.swap(false, Ordering::AcqRel));
        assert!(!notice.swap(false, Ordering::AcqRel));
    }
}
