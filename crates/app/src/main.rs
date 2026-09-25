//! terminator-rust: a terminator-style terminal multiplexer front-end.

mod actions;
mod app_icon;
mod input;
mod ipc;
mod persist;
mod render;
mod session_map;
mod state;
mod ui;
mod windows;

use egui::ViewportBuilder;
use log::warn;

use crate::state::{data, fresh_keep_prefs, fresh_state, AppState, Data};

struct Terminator {
    data: Data,
    path: std::path::PathBuf,
    /// Control socket (UDS) when startup succeeded; None = disabled.
    ipc: Option<ipc::server::Ipc>,
}

impl Terminator {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::fonts::install(&cc.egui_ctx);
        let path = persist::state_path();
        let mut st = launch_state(&path);
        // Compositor-less environments (Xvfb e2e, bare WMs) cannot blend a
        // translucent window: TERMINATOR_OPAQUE=1 pins full opacity so
        // pixels stay deterministic. Startup-only; the inspector slider
        // still works afterwards.
        if std::env::var_os("TERMINATOR_OPAQUE").as_deref() == Some(std::ffi::OsStr::new("1")) {
            st.settings.opacity = 1.0;
        }
        let reg_path = remote::default_registry_path();
        let registry = remote::load_registry(&reg_path);
        Self {
            data: data(st, registry),
            path,
            ipc: ipc::server::start(),
        }
    }

    fn save_if_dirty(&mut self) {
        if !self.data.dirty {
            return;
        }
        if let Err(e) = persist::save(&self.path, &self.data.st) {
            warn!("save state: {e}");
        }
        self.data.dirty = false;
    }
}

impl eframe::App for Terminator {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Keep the WM max-size constraint glued to the real GPU limit
        // (root viewport; secondaries guard inside their own pass).
        render::surface_guard::apply(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_if_dirty();
        }

        let theme = self.data.st.theme_name.clone();
        let font_size = self.data.st.settings.font_size;
        ui::style::sync(&ctx, &theme, font_size, &mut self.data.ui);

        // The control socket and per-window passes all act on the window
        // the user is focused on (set inside windows::render).
        self.data.st.active = self.data.st.focus;
        if let Some(i) = self.ipc.as_mut() {
            ipc::server::drain(i, &mut self.data);
        }

        // Root window: a plain render pass into the root viewport.
        windows::render(ui, &mut self.data, 0);
        ui::inspector::show(&ctx, &mut self.data, 0);
        // Registered AFTER the inspector so the modal sits on top of it.
        ui::close_dialog::show(&ctx, &mut self.data, 0);

        // Secondary windows: immediate viewports = real OS windows, one
        // render pass each (their keyboard input arrives in their own
        // pass inside windows::render). Removal shrinks the list; the loop
        // re-checks the slot so no window is skipped.
        windows::render_secondaries(&ctx, &mut self.data);
        self.data.st.active = self.data.st.focus;

        self.save_if_dirty();

        // Closing the last pane/tab of the last window flips ui.quitting:
        // ask the WM to close every frame until it lands (the close path
        // re-enters ui() once to save; ensure_sessions must not resurrect
        // a tab in between). Secondaries closing never sets this.
        if self.data.ui.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Transparent clear: the window is borderless and the panel fills are
    /// alpha-blended by `settings.opacity`, so the compositor shows the
    /// desktop through. Pixels are fully covered by the panels either way.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }
}

/// Startup state: a fresh window with a NEW tab by default (terminals
/// don't resurrect their last session on open); theme + settings still
/// carry over from the saved state. TERMINATOR_RESTORE=1 keeps the saved
/// windows/tabs instead (e2e presets and users who want it).
fn launch_state(path: &std::path::Path) -> AppState {
    let saved = persist::load(path).unwrap_or_else(fresh_state);
    if std::env::var_os("TERMINATOR_RESTORE").as_deref() == Some(std::ffi::OsStr::new("1")) {
        return saved;
    }
    fresh_keep_prefs(saved)
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // Decode the embedded logo up front (one shared raster for every
    // viewport); a decode failure only costs the custom icon, never the
    // app - app_icon warns and the builders fall back to the default.
    if let Some(icon) = app_icon::icon() {
        log::debug!("window icon: {}x{} rgba", icon.width, icon.height);
    }
    let options = eframe::NativeOptions {
        // The embedded logo (assets/logo, gen-logo.py) becomes the window
        // icon: eframe publishes it as X11 _NET_WM_ICON / the macOS dock
        // icon (see app_icon - secondaries need their own copy).
        viewport: app_icon::with_icon(
            ViewportBuilder::default()
                .with_title("terminator-rust")
                .with_inner_size([1200.0, 800.0])
                // WM-level cap: a window whose PHYSICAL size exceeds the
                // GPU max texture extent aborts the process inside wgpu
                // surface configure (8192 on software adapters; 5K at 2x
                // scale already crosses it). surface_guard::apply
                // refines this to the real adapter limit each frame.
                .with_max_inner_size(render::surface_guard::safe_cap_points(
                    render::surface_guard::FALLBACK_MAX_TEXTURE_SIDE,
                    1.0,
                ))
                // Borderless by design (Chrome-style): the tab strip IS the
                // title bar - dragging the bare chrome moves the window via
                // ViewportCommand::StartDrag (winit's cross-platform
                // drag_window); closing via Ctrl+Shift+Q or the last pane.
                // `with_transparent` stays for window opacity + pane glass.
                .with_decorations(false)
                .with_transparent(true),
        ),
        ..Default::default()
    };
    eframe::run_native(
        "terminator-rust",
        options,
        Box::new(|cc| Ok(Box::new(Terminator::new(cc)))),
    )
}
