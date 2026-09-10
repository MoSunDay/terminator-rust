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
        let st = persist::load(&path).unwrap_or_else(fresh_state);
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
        let chrome = render::colors::to_c32(render::colors::chrome_bg(&pal));
        let page_bg = render::colors::to_c32(pal.background);
        egui::Panel::top("tab_bar")
            .frame(egui::Frame::NONE.fill(chrome))
            .show(ui, |ui| ui::tabs::bar(ui, &mut self.data));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(page_bg))
            .show(ui, |panel| render::screen::screen(panel, &mut self.data));
        ui::inspector::show(&ctx, &mut self.data);

        self.save_if_dirty();
    }
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("terminator-rust")
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "terminator-rust",
        options,
        Box::new(|cc| Ok(Box::new(Terminator::new(cc)))),
    )
}
