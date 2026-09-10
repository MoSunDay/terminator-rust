//! terminator-rust: a terminator-style terminal multiplexer front-end.

mod actions;
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

use crate::state::{data, fresh_state, Data};

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
        let mut st = persist::load(&path).unwrap_or_else(fresh_state);
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
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_if_dirty();
        }

        let theme = self.data.st.theme_name.clone();
        ui::style::sync(&ctx, &theme, &mut self.data.ui);

        // The control socket and per-window passes all act on the window
        // the user is focused on (set inside windows::render).
        self.data.st.active = self.data.st.focus;
        if let Some(i) = self.ipc.as_mut() {
            ipc::server::drain(i, &mut self.data);
        }

        // Root window: a plain render pass into the root viewport.
        windows::render(ui, &mut self.data, 0);
        ui::inspector::show(&ctx, &mut self.data, 0);

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

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("terminator-rust")
            .with_inner_size([1200.0, 800.0])
            // Borderless: no system title bar; dragging happens on the
            // chrome (tab bar / pane headers), closing via Ctrl+Shift+Q,
            // the taskbar, or closing the last pane.
            .with_decorations(false)
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        "terminator-rust",
        options,
        Box::new(|cc| Ok(Box::new(Terminator::new(cc)))),
    )
}
