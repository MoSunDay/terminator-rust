//! Optional wgpu present-mode override for performance experiments.
//!
//! The default is egui-wgpu's `HIGH_THROUGHPUT` surface config
//! (`AutoVsync`, two frames in flight). Setting
//! `TERMINATOR_PRESENT_MODE` chooses another mode before the surface is
//! created; unknown values fall back to the default. This exists to
//! measure whether vblank/present waits dominate on a software or
//! headless host - it is not a user-facing setting.
//!
//! Only wgpu's `Auto*` modes are safe to request before the surface
//! capabilities are known: wgpu-core substitutes a supported fallback
//! for those, while a raw Mailbox/FifoRelaxed request makes
//! `Surface::configure` panic when the backend does not list it. The
//! parser therefore maps every accepted value to an `Auto*` chain; the
//! chain still prefers the requested behavior when the surface supports
//! it (see each arm).
//!
//! The parser is pure and unit-tested; [`configuration`] is the only
//! process-env reader.

use eframe::egui_wgpu;
use eframe::wgpu::PresentMode;

/// Env var selecting the surface present mode.
pub const ENV: &str = "TERMINATOR_PRESENT_MODE";

/// Parse a present mode. `None` means "keep the default".
///
/// Every result is an `Auto*` mode: requesting a raw mode that the
/// surface does not support is a hard wgpu configure error, and the
/// capabilities are unknown until after the first surface exists.
pub fn parse(raw: &str) -> Option<PresentMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto-vsync" | "vsync" | "fifo" => Some(PresentMode::AutoVsync),
        // wgpu's AutoNoVsync prefers Immediate, then Mailbox, then Fifo,
        // so a headless surface that rejects Immediate still works.
        "auto-no-vsync" | "novsync" | "immediate" => Some(PresentMode::AutoNoVsync),
        // The AutoNoVsync chain includes Mailbox as its first fallback
        // after Immediate; this is the closest panic-free request.
        "mailbox" => Some(PresentMode::AutoNoVsync),
        // AutoVsync prefers FifoRelaxed and always ends in Fifo.
        "fifo-relaxed" | "fifo_relaxed" | "relaxed" => Some(PresentMode::AutoVsync),
        _ => None,
    }
}

/// Surface config for the selected mode. The default is byte-for-byte
/// egui-wgpu's high-throughput preset; overrides use one frame of
/// latency so a no-vsync experiment is not masked by a deep queue.
pub fn surface_config(mode: Option<PresentMode>) -> egui_wgpu::SurfaceConfig {
    match mode {
        None => egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT,
        Some(present_mode) => egui_wgpu::SurfaceConfig {
            present_mode,
            desired_maximum_frame_latency: Some(1),
        },
    }
}

/// The wgpu configuration handed to eframe.
pub fn configuration() -> egui_wgpu::WgpuConfiguration {
    let mode = std::env::var(ENV).ok().and_then(|raw| parse(&raw));
    egui_wgpu::WgpuConfiguration::default().with_surface_config(surface_config(mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_modes_parse_and_unknown_keeps_default() {
        assert_eq!(parse("immediate"), Some(PresentMode::AutoNoVsync));
        assert_eq!(parse(" Auto-No-VSync "), Some(PresentMode::AutoNoVsync));
        // Raw Mailbox/FifoRelaxed requests are never emitted: wgpu only
        // falls back for Auto* modes, and an unsupported raw mode panics
        // inside Surface::configure.
        assert_eq!(parse("mailbox"), Some(PresentMode::AutoNoVsync));
        assert_eq!(parse("fifo"), Some(PresentMode::AutoVsync));
        assert_eq!(parse("fifo-relaxed"), Some(PresentMode::AutoVsync));
        assert_eq!(parse("bogus"), None);
    }

    #[test]
    fn default_surface_config_matches_egui_high_throughput() {
        assert_eq!(
            surface_config(None),
            egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT
        );
        let immediate = surface_config(Some(PresentMode::AutoNoVsync));
        assert_eq!(immediate.present_mode, PresentMode::AutoNoVsync);
        assert_eq!(immediate.desired_maximum_frame_latency, Some(1));
    }
}
