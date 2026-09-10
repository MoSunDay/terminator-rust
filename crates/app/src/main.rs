//! terminator-rust: a terminator-style terminal multiplexer front-end.

mod actions;
mod input;
mod ipc;
mod persist;
mod render;
mod session_map;
mod state;
mod ui;

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

        input::keyboard::handle(
            &ctx,
            &mut self.data.st,
            &mut self.data.sess,
            &mut self.data.ui,
            &mut self.data.dirty,
        );
        let theme = self.data.st.theme_name.clone();
        ui::style::sync(&ctx, &theme, &mut self.data.ui);

        if let Some(i) = self.ipc.as_mut() {
            ipc::server::drain(i, &mut self.data);
        }

        let pal = render::colors::palette_of(&theme);
        let opacity = self.data.st.settings.opacity;
        let chrome = render::colors::with_opacity(
            render::colors::to_c32(render::colors::chrome_bg(&pal)),
            opacity,
        );
        let page_bg = render::colors::with_opacity(render::colors::to_c32(pal.background), opacity);
        // Single tab: zero chrome up top - the pane header already
        // identifies the pane, so content starts at y=0. Ctrl+Shift+T (or
        // any multi-tab state) brings the bar back.
        if self.data.st.tree.tabs.len() > 1 {
            egui::Panel::top("tab_bar")
                .frame(egui::Frame::NONE.fill(chrome))
                .show(ui, |ui| ui::tabs::bar(ui, &mut self.data));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(page_bg))
            .show(ui, |panel| render::screen::screen(panel, &mut self.data));
        ui::inspector::show(&ctx, &mut self.data);

        self.save_if_dirty();

        // Closing the last pane/tab flips ui.quitting: ask the WM to close
        // every frame until it lands (the close path re-enters ui() once to
        // save; ensure_sessions must not resurrect a tab in between).
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
