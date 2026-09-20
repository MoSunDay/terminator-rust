//! Settings window: theme picker, font size, terminal background (glass),
//! splits, remote connect form and the saved-host registry.

use std::path::Path;

use egui::{ComboBox, Context, TextEdit, Window};
use log::warn;
use remote::{
    default_registry_path, save_registry, suggest_session_name, upsert_target, PaneKind,
    RemoteTarget,
};

use crate::actions;
use crate::state::Data;

fn form_target(f: &crate::state::RemoteForm) -> RemoteTarget {
    let label = f.label.trim().to_string();
    let host = f.host.trim().to_string();
    let user = f.user.trim().to_string();
    let port = f.port.trim().parse::<u16>().ok();
    let session = if f.session.trim().is_empty() {
        let base = if label.is_empty() { &host } else { &label };
        suggest_session_name(base)
    } else {
        suggest_session_name(f.session.trim())
    };
    RemoteTarget {
        label,
        host,
        user: if user.is_empty() { None } else { Some(user) },
        port,
        session_name: session,
    }
}

fn field(ui: &mut egui::Ui, caption: &str, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(format!("{caption}:"));
        ui.add(
            TextEdit::singleline(value)
                .desired_width(150.0)
                .hint_text(hint),
        );
    });
}

/// Small-caps dimmed section title with breathing room around it;
/// replaces ui.heading + separator for visual grouping.
fn section_title(ui: &mut egui::Ui, pal: &theme::Palette, title: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(title.to_uppercase())
            .small()
            .strong()
            .color(crate::render::colors::to_c32(
                crate::render::colors::title_text(pal),
            )),
    );
    ui.add_space(3.0);
}

/// Show the settings window of OS window `idx` when its per-window
/// `inspector` flag is set. Must be called from THAT window's render pass
/// (root pass for the root window, the viewport callback for secondaries):
/// egui::Window layers land in whichever viewport is rendering, so the
/// panel stays with its owner even when several windows have one open at
/// once (the settings it edits are global).
pub fn show(ctx: &Context, d: &mut Data, idx: usize) {
    let Some(win_id) = d.st.windows.get(idx).map(|w| w.id) else {
        return;
    };
    if !d.st.windows.get(idx).is_some_and(|w| w.ui.inspector) {
        return;
    }
    let reg_path = default_registry_path();
    let mut open = true;
    Window::new("Settings")
        // Per-owner id: two windows can each keep the panel open, and
        // their area state (position/size) must not fight over one id.
        .id(egui::Id::new("inspector").with(win_id))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            theme_section(ui, d);
            splits_section(ui, d);
            bg_section(ui, d);
            form_section(ui, d, &reg_path);
            registry_section(ui, d, &reg_path);
        });
    if let Some(w) = d.st.windows.get_mut(idx) {
        w.ui.inspector = open;
    }
}

fn theme_section(ui: &mut egui::Ui, d: &mut Data) {
    let pal = crate::render::colors::palette_of(&d.st.theme_name);
    section_title(ui, &pal, "Appearance");
    let current = d.st.theme_name.clone();
    ui.horizontal(|ui| {
        ui.label("Theme:");
        ComboBox::from_id_salt("theme")
            .selected_text(&current)
            .show_ui(ui, |c| {
                for name in theme::builtin_names() {
                    c.selectable_value(&mut d.st.theme_name, name.to_string(), name);
                }
            });
    });
    if d.st.theme_name != current {
        d.dirty = true;
    }
    // Font size is a global setting (uniform across every window); the
    // slider works on a local copy and writes back on change.
    let mut size = d.st.settings.font_size;
    if ui
        .add(
            egui::Slider::new(&mut size, 10.0..=24.0)
                .fixed_decimals(0)
                .text("font size"),
        )
        .changed()
    {
        d.st.settings.font_size = size;
        d.dirty = true;
        // Font metrics are re-measured every frame; nothing else to do.
    }
}

fn splits_section(ui: &mut egui::Ui, d: &mut Data) {
    section_title(
        ui,
        &crate::render::colors::palette_of(&d.st.theme_name),
        "Splits",
    );
    // Settings is Copy: edit a copy, then write back and flag dirty when it
    // actually changed (borrowck-friendly, covers combo box AND slider).
    let mut s = d.st.settings;
    ui.horizontal(|ui| {
        ui.label("Default direction:");
        let selected = match s.split_axis {
            layout_tree::Axis::Vertical => "left | right",
            layout_tree::Axis::Horizontal => "top / bottom",
        };
        egui::ComboBox::from_id_salt("split_axis")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut s.split_axis,
                    layout_tree::Axis::Vertical,
                    "left | right",
                );
                ui.selectable_value(
                    &mut s.split_axis,
                    layout_tree::Axis::Horizontal,
                    "top / bottom",
                );
            });
    });
    ui.horizontal(|ui| {
        ui.label("Window opacity:");
        ui.add(egui::Slider::new(&mut s.opacity, 0.5..=1.0));
    });
    let changed = ui
        .add(
            egui::Slider::new(
                &mut s.split_ratio,
                layout_tree::MIN_RATIO..=layout_tree::MAX_RATIO,
            )
            .text("new pane share"),
        )
        .changed()
        || s != d.st.settings;
    d.st.settings = s;
    if changed {
        d.dirty = true;
    }
}

/// Terminal background (glass): transparency + background override, both
/// global (uniform for every pane). Same edit-a-copy-of-Settings pattern
/// as `splits_section`; the hex field buffer lives in egui's temp memory
/// so a per-frame re-render cannot wipe what the user is typing.
fn bg_section(ui: &mut egui::Ui, d: &mut Data) {
    let pal = crate::render::colors::palette_of(&d.st.theme_name);
    section_title(ui, &pal, "Terminal background");
    let mut s = d.st.settings;
    let mut touched = false;
    if ui
        .add(egui::Slider::new(&mut s.transparency, 0.0..=1.0).text("0 = opaque, 1 = glass"))
        .changed()
    {
        touched = true;
    }
    ui.label("Background override:");
    // Palette swatch row: one click applies the color (and re-seeds the
    // hex field to match, like the old per-pane popup did).
    let buf_id = egui::Id::new("settings_bg_hex");
    let mut buf = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_insert_with(buf_id, || {
            s.bg_color.map(theme::rgb_to_hex).unwrap_or_default()
        })
        .clone()
    });
    ui.horizontal_wrapped(|ui| {
        for sw in crate::render::colors::swatches(&pal) {
            let fill = crate::render::colors::to_c32(sw);
            if ui
                .add(egui::Button::new("  ").fill(fill))
                .on_hover_text(theme::rgb_to_hex(sw))
                .clicked()
            {
                s.bg_color = Some(sw);
                buf = theme::rgb_to_hex(sw);
                touched = true;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.add(
            TextEdit::singleline(&mut buf)
                .desired_width(80.0)
                .hint_text("#rrggbb"),
        );
        if ui.button("apply").clicked() {
            // Valid hex only; a bad value keeps the previous override.
            if let Ok(rgb) = theme::parse_hex(buf.trim()) {
                s.bg_color = Some(rgb);
                touched = true;
            }
        }
        if ui.button("clear").clicked() {
            s.bg_color = None;
            buf.clear();
            touched = true;
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(buf_id, buf));
    d.st.settings = s;
    if touched {
        d.dirty = true;
    }
}

fn form_section(ui: &mut egui::Ui, d: &mut Data, reg_path: &Path) {
    section_title(
        ui,
        &crate::render::colors::palette_of(&d.st.theme_name),
        "Remote",
    );
    let Data {
        st,
        sess,
        ui: uist,
        registry,
        dirty,
    } = d;
    let f = &mut uist.form;
    field(ui, "label", &mut f.label, "workstation");
    field(ui, "host", &mut f.host, "host or ip");
    field(ui, "user", &mut f.user, "optional");
    field(ui, "port", &mut f.port, "22");
    field(ui, "session", &mut f.session, "auto");
    let ready = !f.host.trim().is_empty();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(ready, egui::Button::new("Connect (new tab)"))
            .clicked()
        {
            let target = form_target(f);
            upsert_target(registry, &target);
            if let Err(e) = save_registry(reg_path, registry) {
                warn!("save registry: {e}");
            }
            actions::do_new_tab(st, sess, PaneKind::Remote(target), dirty);
            f.host.clear();
            f.session.clear();
        }
        if ui.button("Reset").clicked() {
            *f = crate::state::RemoteForm::default();
        }
    });
    if !ready {
        ui.small("host is required");
    }
}

fn registry_section(ui: &mut egui::Ui, d: &mut Data, reg_path: &Path) {
    section_title(
        ui,
        &crate::render::colors::palette_of(&d.st.theme_name),
        "Saved hosts",
    );
    if d.registry.is_empty() {
        ui.small("(none saved)");
        return;
    }
    let mut connect: Option<RemoteTarget> = None;
    let mut forget: Option<usize> = None;
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .show(ui, |ui| {
            for (i, t) in d.registry.iter().enumerate() {
                ui.horizontal(|ui| {
                    let user = t.user.clone().unwrap_or_default();
                    let text = format!(
                        "{} {}@{}:{} [{}]",
                        t.label,
                        user,
                        t.host,
                        t.port.map(|p| p.to_string()).unwrap_or_else(|| "22".into()),
                        t.session_name
                    );
                    ui.label(text);
                    if ui.small_button("connect").clicked() {
                        connect = Some(t.clone());
                    }
                    if ui.small_button("forget").clicked() {
                        forget = Some(i);
                    }
                });
            }
        });
    let Data {
        st,
        sess,
        registry,
        dirty,
        ..
    } = d;
    if let Some(t) = connect {
        actions::do_new_tab(st, sess, PaneKind::Remote(t), dirty);
    }
    if let Some(i) = forget {
        registry.remove(i);
        if let Err(e) = save_registry(reg_path, registry) {
            warn!("save registry: {e}");
        }
    }
}
