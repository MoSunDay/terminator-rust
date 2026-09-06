//! Inspector window: theme picker, font size, remote connect form and the
//! saved-host registry.

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

/// Show the inspector window when `d.ui.inspector` is set.
pub fn show(ctx: &Context, d: &mut Data) {
    if !d.ui.inspector {
        return;
    }
    let reg_path = default_registry_path();
    let mut open = d.ui.inspector;
    Window::new("Inspector")
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            theme_section(ui, d);
            ui.separator();
            form_section(ui, d, &reg_path);
            ui.separator();
            registry_section(ui, d, &reg_path);
        });
    d.ui.inspector = open;
}

fn theme_section(ui: &mut egui::Ui, d: &mut Data) {
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
    if ui
        .add(
            egui::Slider::new(&mut d.ui.font_size, 10.0..=24.0)
                .fixed_decimals(0)
                .text("font size"),
        )
        .changed()
    {
        // Font metrics are re-measured every frame; nothing else to do.
    }
}

fn form_section(ui: &mut egui::Ui, d: &mut Data, reg_path: &Path) {
    ui.heading("New remote tab");
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
    ui.heading("Saved hosts");
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
