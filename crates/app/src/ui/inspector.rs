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

/// Show the inspector window of OS window `idx` when its per-window
/// `inspector` flag is set. Must be called from THAT window's render pass
/// (root pass for the root window, the viewport callback for secondaries):
/// egui::Window layers land in whichever viewport is rendering, so the
/// panel - and its per-window font slider - stay with their owner even
/// when several windows have it open at once.
pub fn show(ctx: &Context, d: &mut Data, idx: usize) {
    let Some(win_id) = d.st.windows.get(idx).map(|w| w.id) else {
        return;
    };
    if !d.st.windows.get(idx).is_some_and(|w| w.ui.inspector) {
        return;
    }
    let reg_path = default_registry_path();
    let mut open = true;
    Window::new("Inspector")
        // Per-owner id: two windows can each keep an inspector open, and
        // their area state (position/size) must not fight over one id.
        .id(egui::Id::new("inspector").with(win_id))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            theme_section(ui, d, idx);
            splits_section(ui, d);
            form_section(ui, d, &reg_path);
            registry_section(ui, d, &reg_path);
        });
    if let Some(w) = d.st.windows.get_mut(idx) {
        w.ui.inspector = open;
    }
}

fn theme_section(ui: &mut egui::Ui, d: &mut Data, idx: usize) {
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
    // Font size lives in the (per-window) WindowUi of the window that
    // owns this panel; slider works on a local copy and writes back on
    // change.
    let mut size =
        d.st.windows
            .get(idx)
            .map(|w| w.ui.font_size)
            .unwrap_or(14.0);
    if ui
        .add(
            egui::Slider::new(&mut size, 10.0..=24.0)
                .fixed_decimals(0)
                .text("font size"),
        )
        .changed()
    {
        if let Some(w) = d.st.windows.get_mut(idx) {
            w.ui.font_size = size;
        }
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
