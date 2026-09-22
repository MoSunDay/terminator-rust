//! Unit tests for state persistence: JSON round-trips, disk I/O,
//! window-id validation and the corrupt-file backup.

use super::*;
use crate::state::split_tree_pane;
use layout_tree::Axis;

fn sample() -> AppState {
    let mut st = fresh_state();
    st.theme_name = "terminator-classic".to_string();
    let _ = split_tree_pane(&mut st, 0, 1, Axis::Horizontal);
    let _ = split_tree_pane(&mut st, 0, 1, Axis::Vertical);
    if let Some(m) = st.panes.get_mut(&1) {
        m.manual_title = Some("editor".to_string());
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
    assert_eq!(
        st2.windows[0].tree.tabs.len(),
        st.windows[0].tree.tabs.len()
    );
    assert_eq!(
        st2.windows[0].tree.tabs[0].title,
        st.windows[0].tree.tabs[0].title
    );
    assert_eq!(
        layout_tree::pane_count(&st2.windows[0].tree.tabs[0].root),
        3
    );
    assert!(st2.panes.contains_key(&st2.windows[0].tree.tabs[0].focused));
    let ids = layout_tree::sorted_pane_ids(&st2.windows[0].tree.tabs[0].root);
    let m1 = ids
        .iter()
        .find(|id| st2.panes.get(*id).is_some_and(|m| m.manual_title.is_some()));
    assert!(m1.is_some(), "manual title survived");
    let m1 = m1.copied().unwrap_or(0);
    let meta = st2.panes.get(&m1).unwrap_or(&st2.panes[&1]);
    assert_eq!(meta.manual_title.as_deref(), Some("editor"));
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
        layout_tree::pane_count(&back.windows[0].tree.tabs[0].root),
        layout_tree::pane_count(&st.windows[0].tree.tabs[0].root)
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
        windows: vec![],
    };
    let st = from_persisted(&p);
    assert_eq!(st.windows.len(), 1);
    assert_eq!(st.windows[0].tree.tabs.len(), 1);
}

#[test]
fn multi_tab_roundtrip() {
    let mut st = fresh_state();
    let _ = split_tree_pane(&mut st, 0, 1, Axis::Vertical);
    let tree = &mut st.windows[0].tree;
    let idx = layout_tree::new_tab(tree, "second");
    let pane = tree.tabs.get(idx).map(|t| t.focused).unwrap_or(0);
    st.panes.insert(pane, new_pane_meta(PaneKind::Local));
    let back = from_persisted(&to_persisted(&st));
    assert_eq!(back.windows[0].tree.tabs.len(), 2);
    let n0 = layout_tree::pane_count(&back.windows[0].tree.tabs[0].root);
    let n1 = layout_tree::pane_count(&back.windows[0].tree.tabs[1].root);
    assert_eq!((n0, n1), (2, 1));
    assert_ne!(
        back.windows[0].tree.tabs[0].root,
        back.windows[0].tree.tabs[1].root
    );
}

#[test]
fn settings_roundtrip_and_clamp() {
    let mut st = fresh_state();
    st.settings.split_axis = Axis::Horizontal;
    st.settings.opacity = 0.9;
    st.settings.font_size = 12.0;
    st.settings.transparency = 0.5;
    st.settings.bg_color = Some(theme::Rgb {
        r: 0x20,
        g: 0x30,
        b: 0x40,
    });
    let back = from_persisted(&to_persisted(&st));
    assert_eq!(back.settings.split_axis, Axis::Horizontal);
    assert_eq!(back.settings.opacity, 0.9);
    // Global appearance survives save/load with the rest.
    assert_eq!(back.settings.font_size, 12.0);
    assert!((back.settings.transparency - 0.5).abs() < 1e-6);
    assert_eq!(back.settings.bg_color, st.settings.bg_color);
    // Out-of-range values clamp into their sliders' bounds.
    let mut p = to_persisted(&st);
    p.settings.opacity = 0.0;
    p.settings.font_size = 99.0;
    p.settings.transparency = 7.0;
    p.settings.bg = Some("nope".to_string());
    let clamped = from_persisted(&p).settings;
    assert_eq!(clamped.opacity, 0.1);
    assert_eq!(clamped.font_size, 24.0);
    assert_eq!(clamped.transparency, 1.0);
    assert_eq!(clamped.bg_color, None, "unparseable bg -> theme bg");
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
    // The meta keeps legacy per-pane "bg"/"transparency" keys on
    // purpose: serde ignores them now, proving old files still load.
    let lone = serde_json::json!({
        "title": "extra",
        "focused": 1,
        "root": { "Pane": { "id": 3, "meta": {
            "kind": "Local", "manual_title": null,
            "bg": null, "transparency": 0.0, "degraded": false } } }
    });
    obj["windows"].as_array_mut().unwrap()[0]["tabs"]
        .as_array_mut()
        .unwrap()
        .push(lone);
    let back: Persisted = serde_json::from_value(v).unwrap();
    let st2 = from_persisted(&back);
    assert_eq!(st2.windows[0].tree.tabs.len(), 2);
    let extra = &st2.windows[0].tree.tabs[1];
    assert!(layout_tree::contains_pane(&extra.root, extra.focused));
    assert_ne!(extra.focused, st2.windows[0].tree.tabs[0].focused);
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

/// Pre-multi-window state.json: a bare tab list (no `windows` key)
/// loads as one implicit window.
#[test]
fn legacy_tabs_load_as_single_window() {
    let mut st = sample();
    let tree = &mut st.windows[0].tree;
    let idx = layout_tree::new_tab(tree, "legacy second");
    let pane = tree.tabs.get(idx).map(|t| t.focused).unwrap_or(0);
    st.panes.insert(pane, new_pane_meta(PaneKind::Local));
    let mut v = serde_json::to_value(to_persisted(&st)).unwrap();
    if let Some(o) = v.as_object_mut() {
        o.remove("windows");
    }
    let p: Persisted = serde_json::from_value(v).unwrap();
    assert!(p.windows.is_empty(), "file is legacy-shaped");
    let back = from_persisted(&p);
    assert_eq!(back.windows.len(), 1);
    assert_eq!(back.windows[0].id, 1);
    assert_eq!(back.windows[0].tree.tabs.len(), 2);
    assert_eq!(back.active, 0);
    assert_eq!(back.next_window_id, 2);
}

/// Two windows round-trip with globally unique pane ids and per-window
/// tab trees; the legacy `tabs` mirror still tracks window 1.
#[test]
fn two_window_roundtrip_keeps_pane_ids_unique() {
    let mut st = fresh_state();
    let _ = split_tree_pane(&mut st, 0, 1, Axis::Vertical);
    // Second window: seed its allocator above window 1's ids.
    st.windows.push(WindowState {
        id: 7,
        tree: {
            let mut t = layout_tree::new_tree("win2");
            layout_tree::close_tab(&mut t, 0);
            layout_tree::ensure_next_pane_id(&mut t, st.next_pane_id);
            layout_tree::new_tab(&mut t, "win2");
            t
        },
        ui: window_ui(),
    });
    let w2_pane = st.windows[1].tree.tabs[0].focused;
    st.panes.insert(w2_pane, new_pane_meta(PaneKind::Local));
    let p = to_persisted(&st);
    assert_eq!(p.windows.len(), 2);
    assert_eq!(p.tabs.len(), p.windows[0].tabs.len(), "legacy mirror");

    let back = from_persisted(&to_persisted(&st));
    assert_eq!(back.windows.len(), 2);
    assert_eq!(back.windows[0].id, 1);
    assert_eq!(back.windows[1].id, 7);
    assert_eq!(back.active, 0);
    assert_eq!(back.next_window_id, 8);
    let mut all = Vec::new();
    for w in &back.windows {
        for t in &w.tree.tabs {
            layout_tree::pane_ids(&t.root, &mut all);
        }
    }
    let n = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), n, "pane ids unique across windows");
    assert_eq!(back.panes.len(), n, "every rebuilt pane has metadata");
    assert!(back.next_pane_id > *all.iter().max().unwrap_or(&0));
}
/// A persisted window with zero tabs restores as an EMPTY tree for the
/// renderer's respawn path. The old seeded `new_tree` here planted a
/// pane id that no session map ever owned: no respawn, no session,
/// `ctl list` empty - the "opens to nothing" window.
#[test]
fn empty_window_restores_as_empty_tree_for_respawn() {
    let p: Persisted = serde_json::from_str(
        r#"{"theme":"dracula","windows":[{"id":1,"active_tab":0,"tabs":[]}]}"#,
    )
    .unwrap();
    let back = from_persisted(&p);
    assert_eq!(back.windows.len(), 1);
    assert!(back.windows[0].tree.tabs.is_empty());
    assert!(crate::state::all_pane_ids(&back).is_empty());
    assert!(back.panes.is_empty());
    assert_eq!(back.next_pane_id, 2);
}

/// Hand-edited files may carry window ids that break the renderer's
/// invariants: 0 == ViewportId::ROOT, duplicates, u64::MAX overflow.
/// They drop instead of poisoning the whole restore.
#[test]
fn invalid_window_ids_are_dropped() {
    let p: Persisted = serde_json::from_str(
        r#"{"theme":"dracula","windows":[
            {"id":0,"active_tab":0,"tabs":[]},
            {"id":3,"active_tab":0,"tabs":[]},
            {"id":3,"active_tab":0,"tabs":[]},
            {"id":18446744073709551615,"active_tab":0,"tabs":[]}
        ]}"#,
    )
    .unwrap();
    let back = from_persisted(&p);
    assert_eq!(back.windows.len(), 1, "only the valid id 3 survives");
    assert_eq!(back.windows[0].id, 3);
    assert_eq!(back.next_window_id, 4, "computed from survivors only");
}

#[test]
fn all_invalid_windows_fall_back_to_fresh() {
    let p: Persisted = serde_json::from_str(
        r#"{"theme":"dracula","windows":[{"id":0,"active_tab":0,"tabs":[]}]}"#,
    )
    .unwrap();
    let back = from_persisted(&p);
    assert_eq!(back.windows.len(), 1);
    assert_eq!(back.windows[0].id, 1);
}

/// An unparseable state file must be backed up before the fresh-state
/// fallback overwrites it on the next save.
#[test]
fn corrupt_file_is_backed_up() {
    let dir = tempdir("corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.json");
    std::fs::write(&path, b"{ not json").unwrap();
    assert!(load(&path).is_none());
    let backup = dir.join("state.json.corrupt");
    let raw = std::fs::read_to_string(&backup).unwrap_or_default();
    assert_eq!(raw, "{ not json", "offending bytes preserved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_file_makes_no_backup() {
    let dir = tempdir("missing");
    let path = dir.join("state.json");
    assert!(load(&path).is_none());
    assert!(!dir.join("state.json.corrupt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The tmp file is pid-suffixed and never survives the save.
#[test]
fn save_leaves_no_tmp_leftovers() {
    let dir = tempdir("savetmp");
    let path = dir.join("state.json");
    assert!(save(&path, &sample()).is_ok());
    assert!(path.exists());
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
                .collect()
        })
        .unwrap_or_default();
    assert!(leftovers.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
