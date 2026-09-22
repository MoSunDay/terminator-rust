//! Persistence of `AppState` to `~/.terminator-rust/state.json`.
//!
//! The JSON model mirrors the pane tree (without pane ids, which are
//! re-allocated on load and remapped) plus per-pane metadata and the theme.
//! Saves are atomic: write a pid-suffixed temp file, fsync it, then rename
//! it over the target; an unparseable file is backed up before the
//! fresh-state fallback overwrites it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use layout_tree::{
    close_tab, empty_tree, ensure_next_pane_id, new_tab, new_tree, next_pane_id, set_parent_ratio,
    split_pane, Axis, LayoutTree, Node, PaneId,
};
use log::warn;
use remote::{PaneKind, RemoteTarget};
use serde::{Deserialize, Serialize};

use crate::state::{fresh_state, new_pane_meta, window_ui, AppState, PaneMeta, WindowState};

mod settings;

use settings::PSettings;

// ---------------------------------------------------------------------------
// JSON model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Persisted {
    theme: String,
    /// Legacy single-window layout (pre multi-window files): the tabs of
    /// the implicit first window. Still written as a mirror of window 1 so
    /// older binaries keep loading new files.
    #[serde(default)]
    tabs: Vec<PTab>,
    #[serde(default)]
    settings: PSettings,
    /// One entry per OS window (new format); empty in legacy files.
    #[serde(default)]
    windows: Vec<PWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PWindow {
    id: u64,
    #[serde(default)]
    active_tab: usize,
    tabs: Vec<PTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PTab {
    title: String,
    focused: Option<PaneId>,
    root: PNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum PNode {
    Pane {
        id: PaneId,
        meta: PMeta,
    },
    Split {
        axis: PAxis,
        ratio: f32,
        first: Box<PNode>,
        second: Box<PNode>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum PAxis {
    #[serde(rename = "h")]
    Horizontal,
    #[serde(rename = "v")]
    Vertical,
}

impl PAxis {
    fn of(axis: Axis) -> Self {
        match axis {
            Axis::Horizontal => PAxis::Horizontal,
            Axis::Vertical => PAxis::Vertical,
        }
    }

    fn to_axis(self) -> Axis {
        match self {
            PAxis::Horizontal => Axis::Horizontal,
            PAxis::Vertical => Axis::Vertical,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PMeta {
    kind: PKind,
    manual_title: Option<String>,
    degraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum PKind {
    Local,
    Remote {
        label: String,
        host: String,
        user: Option<String>,
        port: Option<u16>,
        session_name: String,
    },
}

fn pm_from(m: &PaneMeta) -> PMeta {
    PMeta {
        kind: match &m.kind {
            PaneKind::Local => PKind::Local,
            PaneKind::Remote(t) => PKind::Remote {
                label: t.label.clone(),
                host: t.host.clone(),
                user: t.user.clone(),
                port: t.port,
                session_name: t.session_name.clone(),
            },
        },
        manual_title: m.manual_title.clone(),
        degraded: m.degraded,
    }
}

fn pm_to(p: &PMeta) -> PaneMeta {
    let kind = match &p.kind {
        PKind::Local => PaneKind::Local,
        PKind::Remote {
            label,
            host,
            user,
            port,
            session_name,
        } => PaneKind::Remote(RemoteTarget {
            label: label.clone(),
            host: host.clone(),
            user: user.clone(),
            port: *port,
            session_name: session_name.clone(),
        }),
    };
    let mut m = new_pane_meta(kind);
    m.manual_title = p.manual_title.clone();
    m.degraded = p.degraded;
    m
}

// ---------------------------------------------------------------------------
// AppState -> JSON
// ---------------------------------------------------------------------------

fn pn_from(node: &Node, panes: &BTreeMap<PaneId, PaneMeta>) -> PNode {
    match node {
        Node::Pane { id } => {
            let meta = panes
                .get(id)
                .map(pm_from)
                .unwrap_or_else(|| pm_from(&new_pane_meta(PaneKind::Local)));
            PNode::Pane { id: *id, meta }
        }
        Node::Split {
            axis,
            ratio,
            first,
            second,
        } => PNode::Split {
            axis: PAxis::of(*axis),
            ratio: *ratio,
            first: Box::new(pn_from(first, panes)),
            second: Box::new(pn_from(second, panes)),
        },
    }
}

fn to_persisted(st: &AppState) -> Persisted {
    let tabs_of = |tree: &LayoutTree| -> Vec<PTab> {
        tree.tabs
            .iter()
            .map(|t| PTab {
                title: t.title.clone(),
                focused: Some(t.focused),
                root: pn_from(&t.root, &st.panes),
            })
            .collect()
    };
    let windows = st
        .windows
        .iter()
        .map(|w| PWindow {
            id: w.id,
            active_tab: w.tree.active_tab.min(w.tree.tabs.len().saturating_sub(1)),
            tabs: tabs_of(&w.tree),
        })
        .collect();
    Persisted {
        theme: st.theme_name.clone(),
        settings: PSettings::of(&st.settings),
        // Legacy mirror: window 1's tabs for pre-multi-window binaries.
        tabs: st
            .windows
            .first()
            .map(|w| tabs_of(&w.tree))
            .unwrap_or_default(),
        windows,
    }
}

// ---------------------------------------------------------------------------
// JSON -> AppState (ids re-allocated and remapped)
// ---------------------------------------------------------------------------

struct Builder {
    remap: HashMap<PaneId, PaneId>,
    panes: BTreeMap<PaneId, PaneMeta>,
    /// Global pane-id budget: ids handed out so far. Every window's tree
    /// allocator is raised above it before allocating, keeping pane ids
    /// unique ACROSS windows.
    next: PaneId,
}

impl Builder {
    /// Expand `node` at the leaf position `anchor`. Returns the pane id that
    /// ends up at that position.
    fn build(&mut self, tree: &mut LayoutTree, tab: usize, node: &PNode, anchor: PaneId) -> PaneId {
        match node {
            PNode::Pane { id, meta } => {
                self.remap.insert(*id, anchor);
                self.panes.insert(anchor, pm_to(meta));
                anchor
            }
            PNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let Some(new_second) = split_pane(tree, tab, anchor, axis.to_axis()) else {
                    return anchor;
                };
                if let Some(t) = tree.tabs.get_mut(tab) {
                    set_parent_ratio(&mut t.root, anchor, *ratio);
                }
                let _ = self.build(tree, tab, first, anchor);
                self.build(tree, tab, second, new_second)
            }
        }
    }
}

/// Window ids a hand-edited file may not violate: 0 collides with
/// ViewportId::ROOT, duplicates collide with each other, and `u64::MAX`
/// would overflow `next_window_id = max + 1`. Drop such entries (and
/// their tabs) with a warning instead of failing the whole load; the
/// survivor list feeds `next_window_id` so it stays consistent.
fn valid_windows(windows: &[PWindow]) -> Vec<(u64, usize, &[PTab])> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for w in windows {
        let in_range = w.id >= 1 && w.id < u64::MAX;
        if in_range && seen.insert(w.id) {
            out.push((w.id, w.active_tab, w.tabs.as_slice()));
        } else {
            warn!(
                "drop persisted window id {} (0, u64::MAX, or duplicate)",
                w.id
            );
        }
    }
    out
}

fn from_persisted(p: &Persisted) -> AppState {
    // New-format files list windows; legacy files carry a bare tab list
    // that maps onto one implicit window (id 1, first tab active).
    let windows_src: Vec<(u64, usize, &[PTab])> = if p.windows.is_empty() {
        if p.tabs.is_empty() {
            return fresh_state();
        }
        vec![(1, 0, p.tabs.as_slice())]
    } else {
        let src = valid_windows(&p.windows);
        if src.is_empty() {
            return fresh_state();
        }
        src
    };
    let mut b = Builder {
        remap: HashMap::new(),
        panes: BTreeMap::new(),
        next: 2,
    };
    let mut windows = Vec::new();
    for (id, active_tab, tabs) in windows_src {
        windows.push(build_window(id, active_tab, tabs, &mut b));
    }
    let next_pane_id = windows
        .iter()
        .map(|w| layout_tree::next_pane_id(&w.tree))
        .max()
        .unwrap_or(2);
    let next_window_id = windows.iter().map(|w| w.id).max().unwrap_or(1).max(1) + 1;
    AppState {
        windows,
        // Restore focuses the root window; interacting with a secondary
        // window flips `focus` within a frame.
        active: 0,
        focus: 0,
        theme_name: if p.theme.is_empty() {
            fresh_state().theme_name
        } else {
            p.theme.clone()
        },
        settings: p.settings.to_settings(),
        panes: b.panes,
        next_pane_id,
        next_window_id,
    }
}

/// Rebuild one window's tree from its persisted tabs, remapping pane ids
/// into the global id space (see [`Builder::next`]). An empty tab list
/// yields an EMPTY tree: the renderer's respawn path opens a fresh tab
/// into it, which also registers the pane in `st.panes` — a seeded
/// `new_tree` here would plant a pane id no session map ever owned.
fn build_window(id: u64, active_tab: usize, tabs: &[PTab], b: &mut Builder) -> WindowState {
    if tabs.is_empty() {
        return WindowState {
            id,
            tree: empty_tree(),
            ui: window_ui(),
        };
    }
    let mut tree = new_tree(&tabs[0].title);
    if b.next > 2 {
        // Not the first window: new_tree() reserved pane 1, which window 1
        // already owns. Drop that seed tab and re-seed the allocator above
        // the global budget before allocating a fresh one.
        close_tab(&mut tree, 0);
        ensure_next_pane_id(&mut tree, b.next);
        new_tab(&mut tree, &tabs[0].title);
    }
    for (i, pt) in tabs.iter().enumerate() {
        if i > 0 {
            new_tab(&mut tree, &pt.title);
        }
        let Some(anchor) = tree.tabs.get(i).map(|t| t.focused) else {
            continue;
        };
        b.build(&mut tree, i, &pt.root, anchor);
        // The id remap is global across tabs: a stale focused id that
        // names a pane in ANOTHER tab still resolves and would leak keys
        // cross-tab, so only accept ids this tab actually contains.
        let focused = pt
            .focused
            .and_then(|old| b.remap.get(&old).copied())
            .filter(|id| {
                tree.tabs
                    .get(i)
                    .is_some_and(|t| layout_tree::contains_pane(&t.root, *id))
            })
            .or_else(|| first_leaf(&tree, i))
            .unwrap_or(anchor);
        if let Some(t) = tree.tabs.get_mut(i) {
            t.focused = focused;
        }
    }
    // new_tab() leaves the LAST tab active while rebuilding; restore the
    // saved one (legacy files default to 0).
    tree.active_tab = active_tab.min(tree.tabs.len().saturating_sub(1));
    b.next = b.next.max(next_pane_id(&tree));
    WindowState {
        id,
        tree,
        ui: window_ui(),
    }
}

fn first_leaf(tree: &LayoutTree, tab: usize) -> Option<PaneId> {
    let t = tree.tabs.get(tab)?;
    let mut node = &t.root;
    loop {
        match node {
            Node::Pane { id } => return Some(*id),
            Node::Split { first, .. } => node = first,
        }
    }
}

// ---------------------------------------------------------------------------
// Disk I/O
// ---------------------------------------------------------------------------

/// `~/.terminator-rust/state.json` (via the shared `paths` crate, which
/// migrates a legacy `$XDG_CONFIG_HOME`/`~/.config` file forward on first
/// use). No XDG base-dir indirection anymore.
pub fn state_path() -> PathBuf {
    paths::migrate_legacy("state.json")
}

/// Atomically write `st` to `path`: the tmp file is pid-suffixed (two
/// app instances sharing a `$HOME` must not interleave writes into one
/// shared tmp), fsynced before the rename (a crash right after the
/// rename must not leave an empty state.json), then renamed over the
/// target. Never panics.
pub fn save(path: &Path, st: &AppState) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(&to_persisted(st))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(json.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Best-effort backup of an unparseable state file before the caller
/// falls back to fresh state (whose next save would silently overwrite
/// the user's layout): `state.json` -> `state.json.corrupt`. Overwrites
/// any earlier backup so the copies cannot accumulate.
fn backup_corrupt(path: &Path) {
    let mut dst = path.as_os_str().to_os_string();
    dst.push(".corrupt");
    let dst = PathBuf::from(dst);
    if let Err(e) = std::fs::copy(path, &dst) {
        warn!("backup {} -> {}: {e}", path.display(), dst.display());
    }
}

/// Load and rebuild state; `None` when missing or unreadable (logged).
pub fn load(path: &Path) -> Option<AppState> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn!("read {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_str::<Persisted>(&raw) {
        Ok(p) => Some(from_persisted(&p)),
        Err(e) => {
            warn!("parse {}: {e}", path.display());
            backup_corrupt(path);
            None
        }
    }
}

#[cfg(test)]
mod tests;
