//! Secondary OS windows: egui immediate viewports driven from the root
//! window's render pass (eframe native registers the immediate-viewport
//! renderer, so these are real OS windows). Each window owns a
//! [`WindowState`] (its own tab tree + scoped UI state); pane ids stay
//! unique across windows via the global `next_pane_id` budget.

use egui::{ViewportBuilder, ViewportId};

use crate::input;
use crate::render;
use crate::session_map::SessionMap;
use crate::state::{new_pane_meta, window_ui, AppState, Data, WindowState};
use crate::ui;
use layout_tree::PaneId;
use remote::PaneKind;

/// Initial geometry for a spawned window: a cascade offset from the root so
/// both windows stay visible even without a window manager. The shell's OSC
/// title sequence re-titles it as soon as its pane emits one.
pub fn builder_for(w: &WindowState) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(format!("terminator-rust #{}", w.id))
        .with_inner_size([900.0, 600.0])
        .with_position([(60 + w.id * 40) as f32, (40 + w.id * 30) as f32])
        // Match the root shell: native title bar + transparent surface
        // (the pane/chrome fills still apply `settings.opacity`).
        .with_transparent(true)
}

/// Ctrl+Shift+N: append a new OS window with one fresh shell tab. Pane ids
/// come from the global budget - the `new_tree` seed tab is dropped first
/// because its pane id 1 would collide with window 1's pane.
pub fn spawn(st: &mut AppState, sess: &mut SessionMap, dirty: &mut bool) {
    let id = st.next_window_id;
    st.next_window_id += 1;
    let mut tree = layout_tree::new_tree("");
    layout_tree::close_tab(&mut tree, 0);
    layout_tree::ensure_next_pane_id(&mut tree, st.next_pane_id);
    let tab = layout_tree::new_tab(&mut tree, "shell");
    st.windows.push(WindowState {
        id,
        tree,
        ui: window_ui(),
    });
    st.collect_alloc();
    let wi = st.windows.len() - 1;
    let Some(pane) = st
        .windows
        .get(wi)
        .and_then(|w| w.tree.tabs.get(tab).map(|t| t.focused))
    else {
        return;
    };
    st.panes.insert(pane, new_pane_meta(PaneKind::Local));
    crate::actions::spawn_pane(st, sess, pane);
    st.focus = wi;
    *dirty = true;
}

/// Drop secondary window `idx` entirely: terminate its panes and fix up
/// focus/active. The root window (0) is only removed by app shutdown.
pub fn remove_window(d: &mut Data, idx: usize) {
    if idx == 0 {
        return;
    }
    let Some(w) = d.st.windows.get(idx) else {
        return;
    };
    let panes: Vec<PaneId> = w
        .tree
        .tabs
        .iter()
        .flat_map(|t| layout_tree::sorted_pane_ids(&t.root))
        .collect();
    for pane in panes {
        d.st.panes.remove(&pane);
        crate::session_map::terminate(&mut d.sess, pane);
    }
    d.st.windows.remove(idx);
    d.st.retarget_after_remove(idx);
    d.dirty = true;
}

/// Render the body of window `idx` into `ui` (root or a secondary
/// viewport): keyboard handling for THIS viewport's input, the tab bar,
/// and the pane area. `st.active` points at `idx` for the duration so
/// every `st.win*()` helper resolves into this window's tree.
pub fn render(ui: &mut egui::Ui, d: &mut Data, idx: usize) {
    let Some(id) = d.st.windows.get(idx).map(|w| w.id) else {
        return;
    };
    // Window outline target: capture the full rect BEFORE the panels
    // below partition it (the border is painted at the end of the pass).
    let win_rect = ui.max_rect();
    d.st.active = idx;
    // Which window does the user consider "the" terminal? The IPC drain and
    // inspector between render passes act on the focused window.
    if ui.ctx().input(|i| i.viewport().focused == Some(true)) {
        d.st.focus = idx;
    }
    input::keyboard::handle(ui.ctx(), &mut d.st, &mut d.sess, &mut d.ui, &mut d.dirty);
    // Closing this window's last pane removes the window mid-pass: stop
    // instead of rendering the next window's tree into this viewport.
    if d.st.windows.get(idx).map(|w| w.id) != Some(id) {
        return;
    }
    let pal = render::colors::palette_of(&d.st.theme_name);
    let opacity = d.st.settings.opacity;
    let chrome = render::colors::with_opacity(
        render::colors::to_c32(render::colors::chrome_bg(&pal)),
        opacity,
    );
    let page_bg = render::colors::with_opacity(render::colors::to_c32(pal.background), opacity);
    // The tab bar is always on (Chrome-style): chips + "+" + the edge
    // cells show even with a single tab.
    egui::Panel::top("tab_bar")
        .frame(egui::Frame::NONE.fill(chrome))
        .show(ui, |ui| ui::tabs::bar(ui, d));
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(page_bg))
        .show(ui, |panel| render::screen::screen(panel, d));
    // IME anchor sync last: screen() just refreshed the focused pane's
    // cursor rect. A no-op while a rename editor owns the keyboard.
    input::ime::sync(ui.ctx(), d, idx);
    // Window-definition hairline: a faint 1px inner edge so the window's
    // extent still reads against a same-colored desktop. Painted last so
    // it sits above every panel fill.
    ui.painter().rect_stroke(
        win_rect,
        0.0,
        egui::Stroke::new(
            1.0,
            render::colors::with_opacity(
                render::colors::to_c32(render::colors::hairline(&pal)),
                opacity,
            ),
        ),
        egui::StrokeKind::Inside,
    );
}

/// Drive every secondary window as an immediate viewport. Removal (WM close
/// or last-pane close) shrinks the list; re-check the slot instead of
/// advancing so no window is skipped.
pub fn render_secondaries(ctx: &egui::Context, d: &mut Data) {
    let mut i = 1;
    while i < d.st.windows.len() {
        let id = d.st.windows[i].id;
        let builder = builder_for(&d.st.windows[i]);
        let mut wm_close = false;
        ctx.show_viewport_immediate(ViewportId(egui::Id::new(id)), builder, |ui, _class| {
            if ui.ctx().input(|inp| inp.viewport().close_requested()) {
                wm_close = true;
            }
            render(ui, d, i);
            // This callback runs as the window's own render pass, so the
            // inspector Window layers into THIS viewport - a panel opened
            // from a secondary stays in (and edits) that secondary. Skip
            // when the pass above removed the window mid-frame.
            if d.st.windows.get(i).map(|w| w.id) == Some(id) {
                ui::inspector::show(ui.ctx(), d, i);
            }
        });
        let gone = d.st.windows.get(i).map(|w| w.id) != Some(id);
        if wm_close && !gone {
            remove_window(d, i);
        } else if !gone {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::fresh_state;

    #[test]
    fn spawn_pushes_window_with_global_pane_ids() {
        let mut st = fresh_state();
        let mut sess = crate::session_map::session_map();
        let mut dirty = false;
        spawn(&mut st, &mut sess, &mut dirty);
        assert_eq!(st.windows.len(), 2);
        assert!(st.windows[1].id >= 2);
        let ids: Vec<PaneId> = st
            .windows
            .iter()
            .flat_map(|w| w.tree.tabs.iter())
            .flat_map(|t| layout_tree::sorted_pane_ids(&t.root))
            .collect();
        assert_eq!(ids.len(), 2, "one pane per window");
        assert_ne!(ids[0], ids[1], "pane ids must be globally unique");
        assert_eq!(st.focus, 1);
        assert!(dirty);
    }

    #[test]
    fn remove_window_terminates_panes_and_retargets() {
        let mut st = fresh_state();
        let mut sess = crate::session_map::session_map();
        let mut dirty = false;
        spawn(&mut st, &mut sess, &mut dirty);
        let gone: Vec<PaneId> = layout_tree::sorted_pane_ids(&st.windows[1].tree.tabs[0].root);
        let mut d = crate::state::data(st, Vec::new());
        remove_window(&mut d, 1);
        assert_eq!(d.st.windows.len(), 1);
        assert_eq!(d.st.focus, 0);
        for p in gone {
            assert!(!d.st.panes.contains_key(&p));
        }
    }
}
