//! Persistence of `AppState` to `~/.config/terminator-rust/state.json`.
//!
//! The JSON model mirrors the pane tree (without pane ids, which are
//! re-allocated on load and remapped) plus per-pane metadata and the theme.
//! Saves are atomic: write a temp file, then rename it over the target.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use layout_tree::{
    new_tab, new_tree, set_parent_ratio, split_pane, Axis, LayoutTree, Node, PaneId, MAX_RATIO,
    MIN_RATIO,
};
use log::warn;
use remote::{PaneKind, RemoteTarget};
use serde::{Deserialize, Serialize};

use crate::state::{fresh_state, new_pane_meta, AppState, PaneMeta};

// ---------------------------------------------------------------------------
// JSON model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Persisted {
    theme: String,
    tabs: Vec<PTab>,
    #[serde(default)]
    settings: PSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PSettings {
    /// "v" = left/right split, "h" = top/bottom.
    split_axis: String,
    split_ratio: f32,
    /// Window opacity; absent in pre-transparency state.json files.
    #[serde(default = "default_opacity")]
    opacity: f32,
}

fn default_opacity() -> f32 {
    // Fully opaque by default: without a compositor (bare X sessions)
    // transparent pixels render BLACK; transparency is opt-in via the
    // inspector slider (clamped 0.5..=1.0).
    1.0
}

impl Default for PSettings {
    fn default() -> Self {
        Self {
            split_axis: "v".to_string(),
            split_ratio: 0.5,
            opacity: default_opacity(),
        }
    }
}

impl PSettings {
    fn of(s: &crate::state::Settings) -> Self {
        Self {
            split_axis: match s.split_axis {
                Axis::Horizontal => "h".to_string(),
                Axis::Vertical => "v".to_string(),
            },
            split_ratio: s.split_ratio,
            opacity: s.opacity,
        }
    }

    fn to_settings(&self) -> crate::state::Settings {
        let axis = if self.split_axis == "h" {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        let ratio = if self.split_ratio.is_finite() {
            self.split_ratio.clamp(MIN_RATIO, MAX_RATIO)
        } else {
            0.5
        };
        let opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.5, 1.0)
        } else {
            default_opacity()
        };
        crate::state::Settings {
            split_axis: axis,
            split_ratio: ratio,
            opacity,
        }
    }
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
    bg: Option<String>,
    transparency: f32,
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
        bg: m
            .bg_color
            .map(|c| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)),
        transparency: m.transparency,
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
    m.bg_color = p.bg.as_deref().and_then(|s| theme::parse_hex(s).ok());
    m.transparency = p.transparency.clamp(0.0, 1.0);
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
    Persisted {
        theme: st.theme_name.clone(),
        settings: PSettings::of(&st.settings),
        tabs: st
            .tree
            .tabs
            .iter()
            .map(|t| PTab {
                title: t.title.clone(),
                focused: Some(t.focused),
                root: pn_from(&t.root, &st.panes),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// JSON -> AppState (ids re-allocated and remapped)
// ---------------------------------------------------------------------------

struct Builder {
    remap: HashMap<PaneId, PaneId>,
    panes: BTreeMap<PaneId, PaneMeta>,
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

fn from_persisted(p: &Persisted) -> AppState {
    if p.tabs.is_empty() {
        return fresh_state();
    }
    let mut tree = new_tree(&p.tabs[0].title);
    let mut b = Builder {
        remap: HashMap::new(),
        panes: BTreeMap::new(),
    };
    for (i, pt) in p.tabs.iter().enumerate() {
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
    // new_tab() leaves the LAST tab active while rebuilding; a restored
    // session should open on the first one.
    tree.active_tab = 0;
    AppState {
        tree,
        theme_name: if p.theme.is_empty() {
            fresh_state().theme_name
        } else {
            p.theme.clone()
        },
        settings: p.settings.to_settings(),
        panes: b.panes,
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

/// `$XDG_CONFIG_HOME/terminator-rust/state.json`, falling back to
/// `$HOME/.config/...`, then to a relative `.config/...` path.
pub fn state_path() -> PathBuf {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".config"),
            _ => PathBuf::from(".config"),
        },
    };
    base.join("terminator-rust").join("state.json")
}

/// Atomically write `st` to `path` (tmp file + rename). Never panics.
pub fn save(path: &Path, st: &AppState) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(&to_persisted(st))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
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
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::split_tree_pane;
    use layout_tree::Axis;
    use theme::Rgb;

    fn sample() -> AppState {
        let mut st = fresh_state();
        st.theme_name = "terminator-classic".to_string();
        let _ = split_tree_pane(&mut st, 0, 1, Axis::Horizontal);
        let _ = split_tree_pane(&mut st, 0, 1, Axis::Vertical);
        if let Some(m) = st.panes.get_mut(&1) {
            m.manual_title = Some("editor".to_string());
            m.bg_color = Some(Rgb {
                r: 0x20,
                g: 0x30,
                b: 0x40,
            });
            m.transparency = 0.5;
        }
        st
    }

    fn tempdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "tr-persist-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn json_roundtrip() {
        let st = sample();
        let p = to_persisted(&st);
        let json = serde_json::to_string(&p).unwrap_or_default();
        let back: Persisted = serde_json::from_str(&json).unwrap_or(p.clone());
        let st2 = from_persisted(&back);
        assert_eq!(st2.theme_name, st.theme_name);
        assert_eq!(st2.tree.tabs.len(), st.tree.tabs.len());
        assert_eq!(st2.tree.tabs[0].title, st.tree.tabs[0].title);
        assert_eq!(layout_tree::pane_count(&st2.tree.tabs[0].root), 3);
        assert!(st2.panes.contains_key(&st2.tree.tabs[0].focused));
        let ids = layout_tree::sorted_pane_ids(&st2.tree.tabs[0].root);
        let m1 = ids
            .iter()
            .find(|id| st2.panes.get(*id).is_some_and(|m| m.manual_title.is_some()));
        assert!(m1.is_some(), "manual title survived");
        let m1 = m1.copied().unwrap_or(0);
        let meta = st2.panes.get(&m1).unwrap_or(&st2.panes[&1]);
        assert_eq!(meta.manual_title.as_deref(), Some("editor"));
        assert_eq!(
            meta.bg_color,
            Some(Rgb {
                r: 0x20,
                g: 0x30,
                b: 0x40
            })
        );
        assert!((meta.transparency - 0.5).abs() < 1e-6);
    }

    #[test]
    fn file_roundtrip() {
        let dir = tempdir("file");
        let path = dir.join("state.json");
        let st = sample();
        assert!(save(&path, &st).is_ok());
        let back = load(&path);
        assert!(back.is_some());
        let back = back.unwrap_or_else(fresh_state);
        assert_eq!(
            layout_tree::pane_count(&back.tree.tabs[0].root),
            layout_tree::pane_count(&st.tree.tabs[0].root)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_none() {
        assert!(load(Path::new("/nonexistent/terminator-rust-test/state.json")).is_none());
    }

    #[test]
    fn empty_tabs_fresh() {
        let p = Persisted {
            theme: "x".to_string(),
            tabs: vec![],
            settings: PSettings::default(),
        };
        let st = from_persisted(&p);
        assert_eq!(st.tree.tabs.len(), 1);
    }

    #[test]
    fn multi_tab_roundtrip() {
        let mut st = fresh_state();
        let _ = split_tree_pane(&mut st, 0, 1, Axis::Vertical);
        let idx = layout_tree::new_tab(&mut st.tree, "second");
        let pane = st.tree.tabs.get(idx).map(|t| t.focused).unwrap_or(0);
        st.panes.insert(pane, new_pane_meta(PaneKind::Local));
        let back = from_persisted(&to_persisted(&st));
        assert_eq!(back.tree.tabs.len(), 2);
        let n0 = layout_tree::pane_count(&back.tree.tabs[0].root);
        let n1 = layout_tree::pane_count(&back.tree.tabs[1].root);
        assert_eq!((n0, n1), (2, 1));
        assert_ne!(back.tree.tabs[0].root, back.tree.tabs[1].root);
    }

    #[test]
    fn settings_roundtrip_and_clamp() {
        let mut st = fresh_state();
        st.settings.split_axis = Axis::Horizontal;
        st.settings.split_ratio = 9.9;
        st.settings.opacity = 0.9;
        let back = from_persisted(&to_persisted(&st));
        assert_eq!(back.settings.split_axis, Axis::Horizontal);
        assert_eq!(back.settings.split_ratio, layout_tree::MAX_RATIO);
        assert_eq!(back.settings.opacity, 0.9);
        // Out-of-range opacity clamps into the slider's bounds.
        let mut p = to_persisted(&st);
        p.settings.opacity = 0.1;
        assert_eq!(from_persisted(&p).settings.opacity, 0.5);
    }

    #[test]
    fn stale_cross_tab_focus_resolves_to_own_leaf() {
        // Tab "extra" is a lone pane(3) but claims focused=1, which only
        // exists in tab 0. The global id remap still resolves 1 -> 1, so
        // only a per-tab containment check keeps keys from leaking into
        // the other tab's pane.
        let mut st = fresh_state();
        let _ = split_tree_pane(&mut st, 0, 1, Axis::Horizontal);
        let p = to_persisted(&st);
        let mut v = serde_json::to_value(p).unwrap();
        let obj = v.as_object_mut().unwrap();
        let lone = serde_json::json!({
            "title": "extra",
            "focused": 1,
            "root": { "Pane": { "id": 3, "meta": {
                "kind": "Local", "manual_title": null,
                "bg": null, "transparency": 0.0, "degraded": false } } }
        });
        obj["tabs"].as_array_mut().unwrap().push(lone);
        let back: Persisted = serde_json::from_value(v).unwrap();
        let st2 = from_persisted(&back);
        assert_eq!(st2.tree.tabs.len(), 2);
        let extra = &st2.tree.tabs[1];
        assert!(layout_tree::contains_pane(&extra.root, extra.focused));
        assert_ne!(extra.focused, st2.tree.tabs[0].focused);
    }

    #[test]
    fn legacy_settings_without_opacity_default() {
        let mut v = serde_json::to_value(to_persisted(&fresh_state())).unwrap();
        if let Some(o) = v.as_object_mut() {
            if let Some(settings) = o.get_mut("settings") {
                settings.as_object_mut().unwrap().remove("opacity");
            }
        }
        let p: Persisted = serde_json::from_value(v).unwrap();
        assert_eq!(from_persisted(&p).settings.opacity, 1.0);
    }

    #[test]
    fn legacy_state_without_settings_uses_defaults() {
        let mut v = serde_json::to_value(to_persisted(&fresh_state())).unwrap();
        if let Some(o) = v.as_object_mut() {
            o.remove("settings");
        }
        let p: Persisted = serde_json::from_value(v).unwrap();
        assert_eq!(
            from_persisted(&p).settings,
            crate::state::Settings::default()
        );
    }
}
