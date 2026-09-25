//! Window-close confirmation dialog: the chrome close X opens a centered
//! modal instead of quitting at the first click. Quit goes out as the
//! ROOT viewport Close, the same path Ctrl+Shift+Q takes.

use crate::render::colors;
use crate::state::Data;
use crate::ui::chrome;

pub const TITLE: &str = "Quit terminator-rust?";

/// Pure body line: how much the quit would take with it. 0 is folded into
/// the singular wording (an empty window list can never reach the dialog).
pub fn quit_hint(window_count: usize) -> String {
    if window_count <= 1 {
        "Closes the window and ends its panes.".to_string()
    } else {
        format!("Closes all {window_count} windows and ends their panes.")
    }
}

/// The one place the exit command is sent from (`ViewportId::ROOT`).
pub(crate) fn request_quit(ctx: &egui::Context) {
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
}

/// Clear the per-window flag (the window can be gone by now).
fn clear(d: &mut Data, idx: usize) {
    if let Some(w) = d.st.windows.get_mut(idx) {
        w.ui.close_dialog = false;
    }
}

/// Show the pending confirmation for window `idx` (no-op if its flag is
/// unset). Must be called from THAT window's render pass - the same rule
/// as `ui::inspector::show` - and AFTER it, so the modal is on top.
pub fn show(ctx: &egui::Context, d: &mut Data, idx: usize) {
    let Some(win_id) = d.st.windows.get(idx).map(|w| w.id) else {
        return;
    };
    if !d.st.windows.get(idx).is_some_and(|w| w.ui.close_dialog) {
        return;
    }
    let m = chrome::metrics(d.st.settings.font_size);
    let pal = colors::palette_of(&d.st.theme_name);
    // Read before the closure: the content cannot borrow `d`.
    let n = d.st.windows.len();
    let resp = egui::Modal::new(egui::Id::new("close_dialog").with(win_id))
        // egui 0.36 has no `Context::style()`: the frame is built from the
        // style of the active theme (the app pins Theme::Dark in
        // ui::style::sync).
        .frame(egui::Frame::popup(&ctx.style_of(ctx.theme())).inner_margin(egui::Margin::same(12)))
        .backdrop_color(egui::Color32::from_black_alpha(96))
        .show(ctx, |ui| {
            // Min AND max: the Area's first-frame available width comes
            // from egui's `Spacing::default_area_size` (600x400) and the
            // right-to-left button row stretches the content rect to its
            // right edge, so a min-width alone left the popup 600 wide
            // (measured by e2e-window-controls R6). One fixed width keeps
            // the frame compact (content + 2x12 margin) and pins the
            // Quit/Cancel anchors the e2e derives from the frame bbox.
            ui.set_min_width(330.0 * m.s);
            ui.set_max_width(330.0 * m.s);
            ui.label(egui::RichText::new(TITLE).strong());
            ui.add_space(4.0 * m.s);
            ui.label(
                egui::RichText::new(quit_hint(n)).color(colors::to_c32(colors::title_text(&pal))),
            );
            ui.add_space(10.0 * m.s);
            let mut quit = false;
            let mut cancel = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // FIRST widget in a right-to-left layout sits flush right.
                let q = ui.add_sized(
                    egui::vec2(96.0 * m.s, 28.0 * m.s),
                    egui::Button::new(
                        egui::RichText::new("Quit").color(colors::to_c32(pal.normal[1])),
                    )
                    // Non-focusable (app-wide rule): a focusable widget
                    // would hand egui's Tab focus around and swallow the
                    // pane keys - see the FOCUS TRAP note in agents.md.
                    .sense(egui::Sense::CLICK),
                );
                quit = q.clicked();
                let c = ui.add_sized(
                    egui::vec2(96.0 * m.s, 28.0 * m.s),
                    egui::Button::new("Cancel").sense(egui::Sense::CLICK),
                );
                cancel = c.clicked();
            });
            (quit, cancel)
        });
    let (quit, cancel) = resp.inner;
    if quit {
        clear(d, idx);
        request_quit(ctx);
    } else if cancel || resp.should_close() {
        // Backdrop click + Esc (egui consumes the Esc only for the
        // topmost modal, so nothing else reacts to it).
        clear(d, idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_hint_single_window() {
        assert_eq!(quit_hint(1), "Closes the window and ends its panes.");
    }

    #[test]
    fn quit_hint_counts_windows() {
        assert_eq!(quit_hint(3), "Closes all 3 windows and ends their panes.");
    }
}
