//! App data model and pure state transitions.
//!
//! No egui types here: geometry uses `layout_tree::Rect`, colors `theme::Rgb`,
//! so everything stays unit-testable headlessly. Session-touching
//! orchestrations live in `actions`.

use std::collections::BTreeMap;

use layout_tree::{new_tree, split_pane, Axis, LayoutTree, PaneId};
use remote::{PaneKind, RemoteTarget};
use theme::Rgb;

/// Pane header strip height in points.
pub const PANE_HEADER_H: f32 = 24.0;
/// Divider thickness (layout_tree default).
pub const DIVIDER_W: f32 = layout_tree::DEFAULT_DIVIDER_W;

/// Per-pane configuration (persisted). Live process state lives in SessionMap.
#[derive(Debug, Clone)]
pub struct PaneMeta {
    pub kind: PaneKind,
    pub manual_title: Option<String>,
    pub bg_color: Option<Rgb>,
    /// 0 = pane bg fully opaque, 1 = pure theme background.
    pub transparency: f32,
    /// Remote pane fell back to a plain ssh shell (no zellij on the host).
    pub degraded: bool,
}

/// Persisted app model: layout tree, per-pane config, theme name.
pub struct AppState {
    pub tree: LayoutTree,
    pub theme_name: String,
    pub panes: BTreeMap<PaneId, PaneMeta>,
}

/// Monospace cell metrics: points for layout, pixels for the pty.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellSize {
    pub w: f32,
    pub h: f32,
    pub w_px: u32,
    pub h_px: u32,
}

/// Divider drag in progress: the pane whose innermost parent split is dragged.
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    pub tab: usize,
    pub pane: PaneId,
}

/// Remote-pane form fields (inspector window).
#[derive(Debug, Clone, Default)]
pub struct RemoteForm {
    pub label: String,
    pub host: String,
    pub user: String,
    pub port: String,
    pub session: String,
}

/// Transient UI state; never persisted.
pub struct UiState {
    pub zoom: bool,
    /// (tab index, edit buffer) while a tab title is being renamed.
    pub tab_edit: Option<(usize, String)>,
    /// (pane id, edit buffer) while a pane title is being renamed.
    pub pane_edit: Option<(PaneId, String)>,
    pub color_open: Option<PaneId>,
    pub color_buf: String,
    pub trans_open: Option<PaneId>,
    pub inspector: bool,
    pub form: RemoteForm,
    pub drag: Option<DragState>,
    pub font_size: f32,
}

pub fn ui_state() -> UiState {
    UiState {
        zoom: false,
        tab_edit: None,
        pane_edit: None,
        color_open: None,
        color_buf: String::new(),
        trans_open: None,
        inspector: false,
        form: RemoteForm::default(),
        drag: None,
        font_size: 14.0,
    }
}

pub fn new_pane_meta(kind: PaneKind) -> PaneMeta {
    PaneMeta {
        kind,
        manual_title: None,
        bg_color: None,
        transparency: 0.0,
        degraded: false,
    }
}

/// Fresh state: one tab, one local shell pane.
pub fn fresh_state() -> AppState {
    let tree = new_tree("shell");
    let mut panes = BTreeMap::new();
    panes.insert(1, new_pane_meta(PaneKind::Local));
    AppState {
        tree,
        theme_name: theme::BUILTIN_NAMES.first().copied().unwrap_or("terminator-classic").to_string(),
        panes,
    }
}

/// All pane ids across all tabs.
pub fn all_pane_ids(tree: &LayoutTree) -> Vec<PaneId> {
    let mut out = Vec::new();
    for tab in &tree.tabs {
        out.extend(layout_tree::sorted_pane_ids(&tab.root));
    }
    out
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

pub fn kind_label(kind: &PaneKind) -> String {
    match kind {
        PaneKind::Local => "shell".to_string(),
        PaneKind::Remote(t) => {
            if t.label.is_empty() {
                t.host.clone()
            } else {
                format!("{}/{}", t.label, t.session_name)
            }
        }
    }
}

/// Title precedence: manual override, then OSC 0/2 title, then kind label.
pub fn effective_title(manual: Option<&str>, osc: &str, kind: &PaneKind) -> String {
    if let Some(m) = manual {
        if !m.trim().is_empty() {
            return m.to_string();
        }
    }
    if !osc.is_empty() {
        return osc.to_string();
    }
    kind_label(kind)
}

/// Terminal grid size for a content area (floored, at least 1x1).
pub fn compute_grid(w: f32, h: f32, cell_w: f32, cell_h: f32) -> (u16, u16) {
    let cw = if cell_w.is_finite() && cell_w >= 1.0 { cell_w } else { 8.0 };
    let ch = if cell_h.is_finite() && cell_h >= 1.0 { cell_h } else { 16.0 };
    let cols = (w / cw).floor().max(1.0).min(f32::from(u16::MAX)) as u16;
    let rows = (h / ch).floor().max(1.0).min(f32::from(u16::MAX)) as u16;
    (cols, rows)
}

// ---------------------------------------------------------------------------
// Shortcut routing (pure table)
// ---------------------------------------------------------------------------

/// Tree/meta half of a split, no process spawning (unit-testable).
/// Returns the new pane id.
pub fn split_tree_pane(
    st: &mut AppState,
    tab: usize,
    pane: PaneId,
    axis: Axis,
) -> Option<PaneId> {
    let new_id = split_pane(&mut st.tree, tab, pane, axis)?;
    st.panes.insert(new_id, new_pane_meta(PaneKind::Local));
    Some(new_id)
}

/// App-level actions triggered by keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewTab,
    SplitHorizontal,
    SplitVertical,
    ClosePane,
    CycleFocus(bool),
    PrevTab,
    NextTab,
    FocusUp,
    FocusDown,
    FocusLeft,
    FocusRight,
    ToggleZoom,
    Respawn,
    CopyNoop,
}

/// Context-menu / header actions on a specific pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneAction {
    SplitHorizontal,
    SplitVertical,
    Close,
    Respawn,
}

/// Shortcut-relevant keys (own enum: no egui dependency).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SKey {
    T,
    O,
    E,
    W,
    R,
    F,
    C,
    V,
    Tab,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
}

/// Modifier snapshot for shortcut routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SMods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Route a keypress to an app action; `None` means "pass to the terminal".
pub fn route_shortcut(m: SMods, k: SKey) -> Option<Action> {
    if !m.ctrl {
        return None;
    }
    match (m.shift, k) {
        (true, SKey::T) => Some(Action::NewTab),
        (true, SKey::O) => Some(Action::SplitHorizontal),
        (true, SKey::E) => Some(Action::SplitVertical),
        (true, SKey::W) => Some(Action::ClosePane),
        (true, SKey::R) => Some(Action::Respawn),
        (true, SKey::F) => Some(Action::ToggleZoom),
        (true, SKey::C) => Some(Action::CopyNoop),
        (true, SKey::Tab) => Some(Action::CycleFocus(false)),
        (false, SKey::Tab) => Some(Action::CycleFocus(true)),
        (true, SKey::ArrowUp) => Some(Action::FocusUp),
        (true, SKey::ArrowDown) => Some(Action::FocusDown),
        (true, SKey::ArrowLeft) => Some(Action::FocusLeft),
        (true, SKey::ArrowRight) => Some(Action::FocusRight),
        (_, SKey::PageUp) => Some(Action::PrevTab),
        (_, SKey::PageDown) => Some(Action::NextTab),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool) -> SMods {
        SMods { ctrl, shift, alt: false }
    }

    #[test]
    fn shortcut_table_matches_spec() {
        assert_eq!(route_shortcut(mods(true, true), SKey::T), Some(Action::NewTab));
        assert_eq!(route_shortcut(mods(true, true), SKey::O), Some(Action::SplitHorizontal));
        assert_eq!(route_shortcut(mods(true, true), SKey::E), Some(Action::SplitVertical));
        assert_eq!(route_shortcut(mods(true, true), SKey::W), Some(Action::ClosePane));
        assert_eq!(route_shortcut(mods(true, true), SKey::F), Some(Action::ToggleZoom));
        assert_eq!(route_shortcut(mods(true, true), SKey::R), Some(Action::Respawn));
        assert_eq!(route_shortcut(mods(false, true), SKey::T), None);
        assert_eq!(route_shortcut(mods(true, false), SKey::V), None);
        assert_eq!(route_shortcut(mods(true, false), SKey::Tab), Some(Action::CycleFocus(true)));
        assert_eq!(route_shortcut(mods(true, true), SKey::Tab), Some(Action::CycleFocus(false)));
        assert_eq!(route_shortcut(mods(true, false), SKey::PageUp), Some(Action::PrevTab));
        assert_eq!(route_shortcut(mods(true, false), SKey::PageDown), Some(Action::NextTab));
        assert_eq!(route_shortcut(mods(true, true), SKey::ArrowLeft), Some(Action::FocusLeft));
        assert_eq!(route_shortcut(mods(true, true), SKey::ArrowDown), Some(Action::FocusDown));
    }

    #[test]
    fn title_precedence() {
        let local = PaneKind::Local;
        assert_eq!(effective_title(Some("work"), "osc", &local), "work");
        assert_eq!(effective_title(Some("  "), "osc", &local), "osc");
        assert_eq!(effective_title(None, "osc", &local), "osc");
        assert_eq!(effective_title(None, "", &local), "shell");
        let t = RemoteTarget {
            label: "box".into(),
            host: "h.example".into(),
            user: None,
            port: None,
            session_name: "work".into(),
        };
        assert_eq!(effective_title(None, "", &PaneKind::Remote(t)), "box/work");
    }

    #[test]
    fn grid_math_floors_and_clamps() {
        assert_eq!(compute_grid(800.0, 600.0, 8.0, 16.0), (100, 37));
        assert_eq!(compute_grid(7.9, 15.9, 8.0, 16.0), (0 + 1, 1));
        assert_eq!(compute_grid(0.0, -5.0, 8.0, 16.0), (1, 1));
        assert_eq!(compute_grid(80.0, 32.0, 0.0, f32::NAN), (10, 2));
    }

    #[test]
    fn split_and_close_keep_maps_consistent() {
        let mut st = fresh_state();
        assert_eq!(all_pane_ids(&st.tree), vec![1]);
        let two = split_tree_pane(&mut st, 0, 1, Axis::Horizontal);
        assert_eq!(two, Some(2));
        assert!(st.panes.contains_key(&2));
        let three = split_tree_pane(&mut st, 0, 2, Axis::Vertical);
        assert_eq!(three, Some(3));
        assert_eq!(layout_tree::pane_count(&st.tree.tabs[0].root), 3);
        assert_eq!(all_pane_ids(&st.tree).len(), 3);
        layout_tree::close_pane(&mut st.tree, 0, 2);
        st.panes.remove(&2);
        assert_eq!(all_pane_ids(&st.tree), vec![1, 3]);
    }
}

// ---------------------------------------------------------------------------
// Composition root
// ---------------------------------------------------------------------------

/// Everything the GUI loop touches: persisted model, live sessions,
/// transient UI state, the saved-host registry and the dirty flag.
pub struct Data {
    pub st: AppState,
    pub sess: crate::session_map::SessionMap,
    pub ui: UiState,
    pub registry: Vec<RemoteTarget>,
    pub dirty: bool,
}

pub fn data(st: AppState, registry: Vec<RemoteTarget>) -> Data {
    Data {
        st,
        sess: crate::session_map::session_map(),
        ui: ui_state(),
        registry,
        dirty: false,
    }
}
