//! Transient per-pane notice aggregation for tab badges.

use std::collections::BTreeSet;

use layout_tree::{PaneId, Tab};

use crate::state::{AppState, UiState};

pub fn tab_has_notice(tab: &Tab, notices: &BTreeSet<PaneId>) -> bool {
    layout_tree::sorted_pane_ids(&tab.root)
        .iter()
        .any(|id| notices.contains(id))
}

pub fn tab_needs_badge(
    tab: &Tab,
    notices: &BTreeSet<PaneId>,
    selected: bool,
    window_focused: bool,
) -> bool {
    tab_has_notice(tab, notices) && !(selected && window_focused)
}

/// Visiting an active tab acknowledges every pane notice in that tab.
pub fn acknowledge_active(st: &AppState, ui: &mut UiState, window: usize) {
    let Some(tab) = st
        .windows
        .get(window)
        .and_then(|w| w.tree.tabs.get(w.tree.active_tab))
    else {
        return;
    };
    for id in layout_tree::sorted_pane_ids(&tab.root) {
        ui.notices.remove(&id);
    }
}

pub fn discard_closed(st: &AppState, ui: &mut UiState) {
    ui.notices.retain(|id| st.panes.contains_key(id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::fresh_state;

    #[test]
    fn repeated_notices_collapse_and_activation_clears() {
        let mut st = fresh_state();
        layout_tree::new_tab(&mut st.windows[0].tree, "other");
        st.windows[0].tree.active_tab = 0;
        st.panes
            .insert(2, crate::state::new_pane_meta(remote::PaneKind::Local));
        let mut ui = crate::state::ui_state();
        ui.notices.insert(2);
        ui.notices.insert(2);
        assert_eq!(ui.notices.len(), 1);
        assert!(tab_has_notice(&st.windows[0].tree.tabs[1], &ui.notices));
        assert!(!tab_has_notice(&st.windows[0].tree.tabs[0], &ui.notices));
        acknowledge_active(&st, &mut ui, 0);
        assert!(ui.notices.contains(&2));
        st.windows[0].tree.active_tab = 1;
        acknowledge_active(&st, &mut ui, 0);
        assert!(ui.notices.is_empty());
    }

    #[test]
    fn split_pane_notice_marks_its_tab_and_closed_panes_are_discarded() {
        let mut st = fresh_state();
        let split =
            layout_tree::split_pane(&mut st.windows[0].tree, 0, 1, layout_tree::Axis::Vertical)
                .unwrap();
        st.panes
            .insert(split, crate::state::new_pane_meta(remote::PaneKind::Local));
        let mut ui = crate::state::ui_state();
        ui.notices.insert(split);
        assert!(tab_has_notice(&st.windows[0].tree.tabs[0], &ui.notices));
        st.panes.remove(&split);
        discard_closed(&st, &mut ui);
        assert!(ui.notices.is_empty());
    }

    #[test]
    fn selected_tab_in_background_window_keeps_its_badge() {
        let st = fresh_state();
        let mut notices = BTreeSet::new();
        notices.insert(1);
        let tab = &st.windows[0].tree.tabs[0];
        assert!(!tab_needs_badge(tab, &notices, true, true));
        assert!(tab_needs_badge(tab, &notices, true, false));
    }
}
