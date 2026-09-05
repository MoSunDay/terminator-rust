//! The tab bar: tab selection, rename, close, and global toggles.

use egui::{Align, Key, Layout, TextEdit, Ui};
use remote::PaneKind;

use crate::actions;
use crate::state::{self, Data};

/// Render the top tab bar. Mutates state via user interactions only.
pub fn bar(ui: &mut Ui, d: &mut Data) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let Data {
            st,
            sess,
            ui: uist,
            dirty,
            ..
        } = d;
        let mut close_tab: Option<usize> = None;
        let count = st.tree.tabs.len();
        for i in 0..count {
            if i > 0 {
                ui.separator();
            }
            let Some(tab) = st.tree.tabs.get(i) else {
                continue;
            };
            let editing = uist
                .tab_edit
                .as_ref()
                .is_some_and(|(a, _)| *a == state::tab_anchor(tab));
            let selected = i == st.tree.active_tab;
            if editing {
                let Some((anchor, buf)) = uist.tab_edit.as_mut() else {
                    continue;
                };
                let resp = ui.add(
                    TextEdit::singleline(buf)
                        .desired_width(110.0)
                        .hint_text("tab title"),
                );
                resp.request_focus();
                let confirm = ui.input(|inp| inp.key_pressed(Key::Enter));
                let cancel = ui.input(|inp| inp.key_pressed(Key::Escape));
                // egui TextEdit keeps focus on Escape, so react to the keys
                // directly instead of waiting for lost_focus.
                if confirm || cancel {
                    if confirm {
                        if let Some(t) = st
                            .tree
                            .tabs
                            .iter_mut()
                            .find(|t| state::tab_anchor(t) == *anchor)
                        {
                            t.title = buf.clone();
                        }
                        *dirty = true;
                    }
                    uist.tab_edit = None;
                }
            } else {
                let panes = layout_tree::pane_count(&tab.root);
                let label = if panes > 1 {
                    format!("{} [{}]", tab.title, panes)
                } else {
                    tab.title.clone()
                };
                let resp = ui.selectable_label(selected, label);
                if resp.clicked() && !selected {
                    st.tree.active_tab = i;
                    uist.zoom = false;
                }
                if resp.double_clicked() {
                    uist.tab_edit = Some((state::tab_anchor(tab), tab.title.clone()));
                }
                if resp.middle_clicked() {
                    close_tab = Some(i);
                }
            }
        }
        if let Some(i) = close_tab {
            actions::do_close_tab(st, sess, uist, i, dirty);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("I").clicked() {
                uist.inspector = !uist.inspector;
            }
            if ui.selectable_label(uist.zoom, "Z").clicked() {
                uist.zoom = !uist.zoom;
            }
            if ui.button("+").clicked() {
                actions::do_new_tab(st, sess, PaneKind::Local, dirty);
            }
        });
    });
    ui.add_space(2.0);
}
