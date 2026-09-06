//! App data model and pure state transitions.
//!
//! No egui types here: geometry uses `layout_tree::Rect`, colors `theme::Rgb`,
//! so everything stays unit-testable headlessly. Session-touching
//! orchestrations live in `actions`.

use std::collections::BTreeMap;

use layout_tree::{new_tree, split_pane_ratio, Axis, LayoutTree, PaneId};
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

/// App-wide settings (persisted).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Axis used by the default split action / new split buttons.
    pub split_axis: Axis,
    /// New pane's share of the split, clamped to 0.05..=0.95.
    pub split_ratio: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            split_axis: Axis::Vertical,
            split_ratio: 0.5,
        }
    }
}

/// Persisted app model: layout tree, per-pane config, theme name.
pub struct AppState {
    pub tree: LayoutTree,
    pub theme_name: String,
    pub settings: Settings,
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

/// Divider drag in progress: the split addressed by (leftmost pane of its
/// first subtree, level from the root).
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    pub tab: usize,
    pub pane: PaneId,
    /// Level of the split along the root->pane path (0 = outermost).
    pub level: usize,
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
    /// (tab anchor pane, edit buffer) while a tab title is being renamed.
    pub tab_edit: Option<(PaneId, String)>,
    /// (pane id, edit buffer) while a pane title is being renamed.
    pub pane_edit: Option<(PaneId, String)>,
    /// Why the current rename was refused (duplicate / digits-only);
    /// shown in red under the editor until accepted or cancelled.
    pub pane_edit_note: Option<&'static str>,
    pub color_open: Option<PaneId>,
    pub color_buf: String,
    pub trans_open: Option<PaneId>,
    pub inspector: bool,
    pub form: RemoteForm,
    pub drag: Option<DragState>,
    /// Pane owning an in-progress pointer drag (selection or motion
    /// reporting) of ANY button; keeps receiving PointerMoved even
    /// outside its rect.
    pub pointer_pane: Option<PaneId>,
    /// Bitmask of held pointer buttons during a grab (bit 0 = Primary,
    /// 1 = Secondary, 2 = Middle, 3 = Extra1, 4 = Extra2).
    pub pointer_buttons: u8,
    /// Bit index of the most recent press still held (motion reports it).
    pub pointer_last: Option<u8>,
    /// Last theme name the egui style was derived from (style::sync memo).
    pub styled_theme: Option<String>,
    /// Modifier state as of the END of the previous frame's input handling;
    /// seed for reconstructing per-event mods (egui 0.36 aggregates
    /// ModifiersChanged into a post-batch `i.modifiers`, which is wrong
    /// for events that landed mid-batch, e.g. a fast ctrl+c whose ctrl
    /// release shares the frame with the folded Event::Copy).
    pub mods_frame_end: egui::Modifiers,
    pub font_size: f32,
}

pub fn ui_state() -> UiState {
    UiState {
        zoom: false,
        tab_edit: None,
        pane_edit: None,
        pane_edit_note: None,
        color_open: None,
        color_buf: String::new(),
        trans_open: None,
        inspector: false,
        form: RemoteForm::default(),
        drag: None,
        pointer_pane: None,
        pointer_buttons: 0,
        pointer_last: None,
        styled_theme: None,
        mods_frame_end: egui::Modifiers::NONE,
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
        theme_name: theme::BUILTIN_NAMES
            .first()
            .copied()
            .unwrap_or("terminator-classic")
            .to_string(),
        settings: Settings::default(),
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

/// Stable identity of a tab: its lowest pane id. Pane ids are unique,
/// never reused and never move between tabs, so this survives index
/// shifts; only closing that specific pane invalidates it.
pub fn tab_anchor(tab: &layout_tree::Tab) -> PaneId {
    layout_tree::sorted_pane_ids(&tab.root)
        .first()
        .copied()
        .unwrap_or(tab.focused)
}

/// True when no live tab carries this anchor: a rename edit buffer
/// targeting it is dead (its anchor pane was closed) and should drop.
/// (Wired by the pane-close path in actions.rs.)
pub fn tab_edit_orphaned(tree: &LayoutTree, anchor: PaneId) -> bool {
    tree.tabs.iter().all(|t| tab_anchor(t) != anchor)
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

/// True when a pane other than `except` already claims this exact manual
/// title: manual titles are the addressing key of the control socket, so
/// they must stay unique.
pub fn manual_title_taken(st: &AppState, title: &str, except: PaneId) -> bool {
    st.panes
        .iter()
        .any(|(id, m)| *id != except && m.manual_title.as_deref() == Some(title))
}

/// Terminal grid size for a content area (floored, at least 1x1).
pub fn compute_grid(w: f32, h: f32, cell_w: f32, cell_h: f32) -> (u16, u16) {
    let cw = if cell_w.is_finite() && cell_w >= 1.0 {
        cell_w
    } else {
        8.0
    };
    let ch = if cell_h.is_finite() && cell_h >= 1.0 {
        cell_h
    } else {
        16.0
    };
    let cols = (w / cw).floor().max(1.0).min(f32::from(u16::MAX)) as u16;
    let rows = (h / ch).floor().max(1.0).min(f32::from(u16::MAX)) as u16;
    (cols, rows)
}

// ---------------------------------------------------------------------------
// Shortcut routing (pure table)
// ---------------------------------------------------------------------------

/// Tree/meta half of a split, no process spawning (unit-testable).
/// Returns the new pane id.
pub fn split_tree_pane(st: &mut AppState, tab: usize, pane: PaneId, axis: Axis) -> Option<PaneId> {
    let ratio = st.settings.split_ratio;
    let new_id = split_pane_ratio(&mut st.tree, tab, pane, axis, ratio)?;
    st.panes.insert(new_id, new_pane_meta(PaneKind::Local));
    Some(new_id)
}

/// Pure shortcut-routing table (re-exported for convenience).
pub mod shortcuts;

pub use shortcuts::{route_shortcut, Action, PaneAction, SKey, SMods};

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn manual_title_taken_ignores_self_and_empty_titles() {
        let mut st = fresh_state();
        st.panes.get_mut(&1).unwrap().manual_title = Some("agent".into());
        assert!(!manual_title_taken(&st, "agent", 1), "own title is fine");
        assert!(!manual_title_taken(&st, "other", 1));
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        st.panes.get_mut(&2).unwrap().manual_title = Some("agent".into());
        assert!(manual_title_taken(&st, "agent", 1));
        assert!(manual_title_taken(&st, "agent", 2));
        assert!(!manual_title_taken(&st, "agent2", 1));
    }

    #[test]
    fn grid_math_floors_and_clamps() {
        assert_eq!(compute_grid(800.0, 600.0, 8.0, 16.0), (100, 37));
        assert_eq!(compute_grid(7.9, 15.9, 8.0, 16.0), (1, 1));
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
    #[test]
    fn tab_anchor_is_the_lowest_pane_id() {
        let mut st = fresh_state();
        assert_eq!(tab_anchor(&st.tree.tabs[0]), 1);
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        // Still pane 1 (lowest id), unaffected by new splits.
        assert_eq!(tab_anchor(&st.tree.tabs[0]), 1);
    }

    #[test]
    fn tab_edit_orphaned_tracks_anchor_pane_close() {
        let mut st = fresh_state();
        assert!(!tab_edit_orphaned(&st.tree, 1));
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        assert!(!tab_edit_orphaned(&st.tree, 1));
        // Closing the anchor pane moves the tab to a new anchor, orphaning
        // any rename buffer keyed on the old one.
        layout_tree::close_pane(&mut st.tree, 0, 1);
        st.panes.remove(&1);
        assert!(tab_edit_orphaned(&st.tree, 1));
        assert_eq!(tab_anchor(&st.tree.tabs[0]), 2);
        assert!(!tab_edit_orphaned(&st.tree, 2));
    }
}
