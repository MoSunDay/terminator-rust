//! Confirmation for the chrome window and tab close buttons.

use crate::actions;
use crate::render::colors;
use crate::state::{self, CloseDialog, Data};
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
        w.ui.close_dialog = None;
    }
}

fn tab_index(tree: &layout_tree::LayoutTree, anchor: layout_tree::PaneId) -> Option<usize> {
    tree.tabs
        .iter()
        .position(|t| state::tab_anchor(t) == anchor)
}

/// Show the pending confirmation for window `idx` (no-op if its flag is
/// unset). Must be called from THAT window's render pass - the same rule
/// as `ui::inspector::show` - and AFTER it, so the modal is on top.
pub fn show(ctx: &egui::Context, d: &mut Data, idx: usize) {
    let Some(win_id) = d.st.windows.get(idx).map(|w| w.id) else {
        return;
    };
    let Some(target) = d.st.windows.get(idx).and_then(|w| w.ui.close_dialog) else {
        return;
    };
    let tab = match target {
        CloseDialog::Quit => None,
        CloseDialog::Tab(anchor) => {
            let Some(tab) = tab_index(&d.st.windows[idx].tree, anchor) else {
                clear(d, idx);
                return;
            };
            Some(tab)
        }
    };
    let m = chrome::metrics(d.st.settings.font_size);
    let pal = colors::palette_of(&d.st.theme_name);
    // Read before the closure: the content cannot borrow `d`.
    let n = d.st.windows.len();
    let (title, hint, action) = match tab {
        Some(tab) => {
            let pane_count = layout_tree::pane_count(&d.st.windows[idx].tree.tabs[tab].root);
            let hint = if pane_count == 1 {
                "Closes this tab and ends its terminal.".to_string()
            } else {
                format!("Closes this tab and ends its {pane_count} terminals.")
            };
            ("Close tab?", hint, "Close tab")
        }
        None => (TITLE, quit_hint(n), "Quit"),
    };
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
            ui.label(egui::RichText::new(title).strong());
            ui.add_space(4.0 * m.s);
            ui.label(egui::RichText::new(hint).color(colors::to_c32(colors::title_text(&pal))));
            ui.add_space(10.0 * m.s);
            let mut quit = false;
            let mut cancel = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // FIRST widget in a right-to-left layout sits flush right.
                let q = ui.add_sized(
                    egui::vec2(96.0 * m.s, 28.0 * m.s),
                    egui::Button::new(
                        egui::RichText::new(action).color(colors::to_c32(pal.normal[1])),
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
    let (confirmed, cancel) = resp.inner;
    if confirmed {
        clear(d, idx);
        if let Some(tab) = tab {
            d.st.active = idx;
            actions::do_close_tab(&mut d.st, &mut d.sess, &mut d.ui, tab, &mut d.dirty);
        } else {
            request_quit(ctx);
        }
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

    #[test]
    fn pending_tab_follows_reorder_and_vanishes_with_its_tab() {
        let mut tree = layout_tree::new_tree("first");
        let target = layout_tree::new_tab(&mut tree, "target");
        let anchor = state::tab_anchor(&tree.tabs[target]);
        assert!(layout_tree::move_tab(&mut tree, target, 0));
        assert_eq!(tab_index(&tree, anchor), Some(0));
        layout_tree::close_tab(&mut tree, 0);
        assert_eq!(tab_index(&tree, anchor), None);
    }
}
