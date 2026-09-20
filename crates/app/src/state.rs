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

/// Default terminal font size (points). 15 makes the embedded Maple
/// Mono advance (0.6em) land on exactly 9 device pixels at 1x scale, so
/// the cell pitch is naturally integer (no snapping loss); 20/25 too.
pub const DEFAULT_FONT_SIZE: f32 = 15.0;

/// Per-pane configuration (persisted). Live process state lives in SessionMap.
/// Pane appearance (bg override, glass) lives in the global
/// [`Settings`] - uniform for every pane.
#[derive(Debug, Clone)]
pub struct PaneMeta {
    pub kind: PaneKind,
    pub manual_title: Option<String>,
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
    /// Window opacity (0.5..=1.0): alpha of the chrome/pane base fills;
    /// text and selections stay opaque for readability. 1.0 = opaque.
    pub opacity: f32,
    /// Terminal font size in points, uniform for every window.
    pub font_size: f32,
    /// Terminal background transparency ("glass"): 0 = opaque, 1 = fully
    /// see-through (the desktop shows through; ink stays solid). Uniform
    /// for all panes.
    pub transparency: f32,
    /// Terminal background color override for all panes; None = theme bg.
    pub bg_color: Option<Rgb>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            split_axis: Axis::Vertical,
            split_ratio: 0.5,
            opacity: 1.0,
            font_size: DEFAULT_FONT_SIZE,
            transparency: 0.0,
            bg_color: None,
        }
    }
}

/// Persisted app model: per-window layout trees, per-pane config, theme.
pub struct AppState {
    /// One entry per OS window; invariant: >= 1 while the app runs.
    /// Tab indices inside each tree are per-window.
    pub windows: Vec<WindowState>,
    /// Focused window index (clamped on use, see [`AppState::active_idx`]).
    pub active: usize,
    /// Window the USER is interacting with (keyboard focus). `active` is the
    /// window currently being rendered; they differ only inside a
    /// non-focused viewport's render pass. Restored to 0 on load.
    pub focus: usize,
    pub theme_name: String,
    pub settings: Settings,
    pub panes: BTreeMap<PaneId, PaneMeta>,
    /// Global pane-id counter: keeps ids unique across all windows.
    pub next_pane_id: PaneId,
    /// Next OS-window id; windows get stable ids starting at 1
    /// (0 is egui's ROOT viewport).
    pub next_window_id: u64,
}

impl AppState {
    pub fn active_idx(&self) -> usize {
        self.active.min(self.windows.len().saturating_sub(1))
    }

    pub fn win(&self) -> Option<&WindowState> {
        self.windows.get(self.active_idx())
    }

    pub fn win_mut(&mut self) -> Option<&mut WindowState> {
        let i = self.active_idx();
        self.windows.get_mut(i)
    }

    /// Fix up `focus` after `windows.remove(removed)`: the removed window
    /// falls back to the root (0), higher indices shift down.
    pub fn retarget_after_remove(&mut self, removed: usize) {
        if self.focus == removed {
            self.focus = 0;
        } else if self.focus > removed {
            self.focus -= 1;
        }
        let n = self.windows.len();
        self.focus = self.focus.min(n.saturating_sub(1));
        self.active = self.active.min(n.saturating_sub(1));
    }

    /// Global pane-id uniqueness: seed a window's tree allocator before a
    /// mutation that allocates (split), then [`AppState::collect_alloc`]
    /// afterwards.
    pub fn seed_alloc(&mut self, win: usize) {
        let next = self.next_pane_id;
        if let Some(w) = self.windows.get_mut(win) {
            layout_tree::ensure_next_pane_id(&mut w.tree, next);
        }
    }

    pub fn collect_alloc(&mut self) {
        let mut next = self.next_pane_id;
        for w in &self.windows {
            next = next.max(layout_tree::next_pane_id(&w.tree));
        }
        self.next_pane_id = next;
    }
}

/// One OS window: its own tab tree plus scoped UI state.
pub struct WindowState {
    /// Stable window id (starts at 1).
    pub id: u64,
    pub tree: LayoutTree,
    pub ui: WindowUi,
}

/// Monospace cell metrics: points for layout, pixels for the pty.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellSize {
    pub w: f32,
    pub h: f32,
    /// Font size wide (double-width CJK) cells are painted at: the CJK
    /// fallback font draws Han at a ~1.0em advance, so it is scaled until
    /// one glyph exactly fills two narrow cells.
    pub wide_size: f32,
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

/// In-flight tab-chip drag: the dragged tab is identified by its stable
/// anchor (lowest pane id - survives index shifts), `grab_dx` anchors the
/// ghost chip under the pointer, `w` is the chip width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabDrag {
    pub anchor: PaneId,
    pub grab_dx: f32,
    pub w: f32,
}

/// In-flight Ctrl+drag pane move: `target` is refreshed every frame from
/// the hovered pane (pane id + drop zone); `dwell` tracks a hover on
/// another tab's chip (tab index + hover start time) that switches the
/// active tab mid-drag so the drop lands in that tab. On release the
/// move executes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneDrag {
    pub pane: PaneId,
    pub target: Option<(PaneId, layout_tree::DropZone)>,
    pub dwell: Option<(usize, f64)>,
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

/// Transient per-OS-window UI state (input stream + editors scoped to one
/// window); never persisted.
pub struct WindowUi {
    pub zoom: bool,
    /// (tab anchor pane, edit buffer) while a tab title is being renamed.
    pub tab_edit: Option<(PaneId, String)>,
    /// (pane id, edit buffer) while a pane title is being renamed.
    pub pane_edit: Option<(PaneId, String)>,
    /// Why the current rename was refused (duplicate / digits-only);
    /// shown in red under the editor until accepted or cancelled.
    pub pane_edit_note: Option<&'static str>,
    pub drag: Option<DragState>,
    /// In-flight tab-chip reorder drag (transient, never persisted).
    pub tab_drag: Option<TabDrag>,
    /// In-flight Ctrl+drag pane move (transient, never persisted).
    pub pane_drag: Option<PaneDrag>,
    /// Pane owning an in-progress pointer drag (selection or motion
    /// reporting) of ANY button; keeps receiving PointerMoved even
    /// outside its rect.
    pub pointer_pane: Option<PaneId>,
    /// Bitmask of held pointer buttons during a grab (bit 0 = Primary,
    /// 1 = Secondary, 2 = Middle, 3 = Extra1, 4 = Extra2).
    pub pointer_buttons: u8,
    /// Bit index of the most recent press still held (motion reports it).
    pub pointer_last: Option<u8>,
    /// Modifier state as of the END of the previous frame's input handling;
    /// seed for reconstructing per-event mods (egui 0.36 aggregates
    /// ModifiersChanged into a post-batch `i.modifiers`, which is wrong
    /// for events that landed mid-batch, e.g. a fast ctrl+c whose ctrl
    /// release shares the frame with the folded Event::Copy).
    pub mods_frame_end: egui::Modifiers,
    /// Settings panel open in THIS window. The panel is drawn in the
    /// owning window's own render pass, so it edits this window's view.
    pub inspector: bool,
    /// Live IME preedit (composition) text for this window; None while
    /// no composition is in flight (cleared on commit and whenever a
    /// text field takes the keyboard).
    pub ime: Option<String>,
    /// Focused pane's cursor cell rect in egui points, refreshed by the
    /// render pass each frame; the IME popup anchors here.
    pub ime_cursor: Option<egui::Rect>,
    /// Pane the IME anchor belonged to last frame; a change interrupts
    /// any in-flight composition.
    pub ime_pane: Option<PaneId>,
    /// Pane the IME platform output was anchored to on the previous
    /// frame (interrupt detection memory).
    pub ime_last_pane: Option<PaneId>,
    /// Chip-strip horizontal scroll offset (px); 0 while the tabs fit.
    pub tab_scroll: f32,
    /// Active tab when the scroll was last auto-followed (keep the active
    /// chip visible on tab switches, but never fight manual scrolling).
    pub tab_scroll_tab: usize,
}

/// Transient app-global UI state; never persisted.
pub struct UiState {
    pub form: RemoteForm,
    /// Last theme name the egui style was derived from (style::sync memo).
    pub styled_theme: Option<String>,
    /// The last pane/tab was closed: the app is shutting down. Guards the
    /// empty-tabs auto-respawn until the ViewportCommand::Close lands.
    pub quitting: bool,
}

pub fn window_ui() -> WindowUi {
    WindowUi {
        zoom: false,
        tab_edit: None,
        pane_edit: None,
        pane_edit_note: None,
        drag: None,
        tab_drag: None,
        pane_drag: None,
        pointer_pane: None,
        pointer_buttons: 0,
        pointer_last: None,
        mods_frame_end: egui::Modifiers::NONE,
        inspector: false,
        ime: None,
        ime_cursor: None,
        ime_pane: None,
        ime_last_pane: None,
        tab_scroll: 0.0,
        tab_scroll_tab: 0,
    }
}

pub fn ui_state() -> UiState {
    UiState {
        form: RemoteForm::default(),
        styled_theme: None,
        quitting: false,
    }
}

pub fn new_pane_meta(kind: PaneKind) -> PaneMeta {
    PaneMeta {
        kind,
        manual_title: None,
        degraded: false,
    }
}

/// Fresh state: one window (id 1), one tab, one local shell pane.
pub fn fresh_state() -> AppState {
    let tree = new_tree("shell");
    let mut panes = BTreeMap::new();
    panes.insert(1, new_pane_meta(PaneKind::Local));
    let next_pane_id = layout_tree::next_pane_id(&tree);
    AppState {
        windows: vec![WindowState {
            id: 1,
            tree,
            ui: window_ui(),
        }],
        active: 0,
        focus: 0,
        theme_name: theme::BUILTIN_NAMES
            .first()
            .copied()
            .unwrap_or("terminator-classic")
            .to_string(),
        settings: Settings::default(),
        panes,
        next_pane_id,
        next_window_id: 2,
    }
}

/// All pane ids across all tabs of all windows, flat and unique.
pub fn all_pane_ids(st: &AppState) -> Vec<PaneId> {
    let mut out: Vec<PaneId> = Vec::new();
    for w in &st.windows {
        for tab in &w.tree.tabs {
            for id in layout_tree::sorted_pane_ids(&tab.root) {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
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
pub fn manual_title_taken(panes: &BTreeMap<PaneId, PaneMeta>, title: &str, except: PaneId) -> bool {
    panes
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
/// Splits in the ACTIVE window's tree; keeps the global pane-id counter in
/// sync. Returns the new pane id.
pub fn split_tree_pane(st: &mut AppState, tab: usize, pane: PaneId, axis: Axis) -> Option<PaneId> {
    let ratio = st.settings.split_ratio;
    let wi = st.active_idx();
    st.seed_alloc(wi);
    let new_id = st
        .windows
        .get_mut(wi)
        .and_then(|w| split_pane_ratio(&mut w.tree, tab, pane, axis, ratio));
    st.collect_alloc();
    let new_id = new_id?;
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
        assert!(
            !manual_title_taken(&st.panes, "agent", 1),
            "own title is fine"
        );
        assert!(!manual_title_taken(&st.panes, "other", 1));
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        st.panes.get_mut(&2).unwrap().manual_title = Some("agent".into());
        assert!(manual_title_taken(&st.panes, "agent", 1));
        assert!(manual_title_taken(&st.panes, "agent", 2));
        assert!(!manual_title_taken(&st.panes, "agent2", 1));
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
        assert_eq!(all_pane_ids(&st), vec![1]);
        let two = split_tree_pane(&mut st, 0, 1, Axis::Horizontal);
        assert_eq!(two, Some(2));
        assert!(st.panes.contains_key(&2));
        let three = split_tree_pane(&mut st, 0, 2, Axis::Vertical);
        assert_eq!(three, Some(3));
        assert_eq!(layout_tree::pane_count(&st.windows[0].tree.tabs[0].root), 3);
        assert_eq!(all_pane_ids(&st).len(), 3);
        layout_tree::close_pane(&mut st.windows[0].tree, 0, 2);
        st.panes.remove(&2);
        assert_eq!(all_pane_ids(&st), vec![1, 3]);
    }
    #[test]
    fn tab_anchor_is_the_lowest_pane_id() {
        let mut st = fresh_state();
        assert_eq!(tab_anchor(&st.windows[0].tree.tabs[0]), 1);
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        // Still pane 1 (lowest id), unaffected by new splits.
        assert_eq!(tab_anchor(&st.windows[0].tree.tabs[0]), 1);
    }

    #[test]
    fn tab_edit_orphaned_tracks_anchor_pane_close() {
        let mut st = fresh_state();
        assert!(!tab_edit_orphaned(&st.windows[0].tree, 1));
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        assert!(!tab_edit_orphaned(&st.windows[0].tree, 1));
        // Closing the anchor pane moves the tab to a new anchor, orphaning
        // any rename buffer keyed on the old one.
        layout_tree::close_pane(&mut st.windows[0].tree, 0, 1);
        st.panes.remove(&1);
        assert!(tab_edit_orphaned(&st.windows[0].tree, 1));
        assert_eq!(tab_anchor(&st.windows[0].tree.tabs[0]), 2);
        assert!(!tab_edit_orphaned(&st.windows[0].tree, 2));
    }

    #[test]
    fn active_idx_clamps_and_win_helpers_stay_option() {
        let mut st = fresh_state();
        assert_eq!(st.active_idx(), 0);
        assert!(st.win().is_some() && st.win_mut().is_some());
        st.active = 9; // out of range: clamped, never panics
        assert_eq!(st.active_idx(), 0);
        st.windows.clear();
        assert_eq!(st.active_idx(), 0);
        assert!(st.win().is_none());
        assert!(st.win_mut().is_none());
    }

    #[test]
    fn seed_and_collect_keep_pane_ids_unique_across_windows() {
        let mut st = fresh_state();
        assert_eq!(split_tree_pane(&mut st, 0, 1, Axis::Vertical), Some(2));
        // A second window joins with a tree allocated from scratch: seed
        // its allocator from the global counter so a new tab cannot reuse
        // ids already live in the first window.
        let mut aux = layout_tree::new_tree("aux");
        layout_tree::close_tab(&mut aux, 0);
        st.windows.push(WindowState {
            id: 2,
            tree: aux,
            ui: window_ui(),
        });
        let wi = st.windows.len() - 1;
        st.seed_alloc(wi);
        let tab = layout_tree::new_tab(&mut st.windows[wi].tree, "aux");
        st.collect_alloc();
        let pane = st.windows[wi].tree.tabs[tab].focused;
        st.panes.insert(pane, new_pane_meta(PaneKind::Local));
        assert_eq!(pane, 3, "fresh id drawn from the global counter");
        let ids = all_pane_ids(&st);
        assert_eq!(ids.len(), 3);
        assert_eq!(
            ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
            3,
            "pane ids stay unique across windows"
        );
        assert_eq!(st.next_pane_id, 4);
    }
}
