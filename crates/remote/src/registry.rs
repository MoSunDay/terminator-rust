//! Remote session registry: a JSON file of known remote targets so a dead
//! pane can be reconnected to the same zellij session with state intact.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use log::warn;
use serde::{Deserialize, Serialize};

/// A user-chosen remote destination plus the zellij session to attach to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteTarget {
    /// User-chosen display label (free form, never sent to a shell).
    pub label: String,
    /// Remote host name or address.
    pub host: String,
    /// SSH user; `None` means "let ssh decide" (ssh_config / current user).
    pub user: Option<String>,
    /// SSH port; `None` means ssh default (22).
    pub port: Option<u16>,
    /// Zellij session name on the remote host (constrained to [a-z0-9-]).
    pub session_name: String,
}

/// `$XDG_CONFIG_HOME/terminator-rust/sessions.json`, falling back to
/// `$HOME/.config/...`, then to a relative `.config/...` path.
pub fn default_registry_path() -> PathBuf {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".config"),
            _ => PathBuf::from(".config"),
        },
    };
    base.join("terminator-rust").join("sessions.json")
}

/// Read the registry. A missing file yields an empty list; so does
/// malformed content (logged as a warning). Never panics.
pub fn load_registry(path: &Path) -> Vec<RemoteTarget> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warn!(
                "remote registry {} unreadable: {e}; starting empty",
                path.display()
            );
            return Vec::new();
        }
    };
    match serde_json::from_str(&raw) {
        Ok(targets) => targets,
        Err(e) => {
            warn!(
                "remote registry {} malformed: {e}; starting empty",
                path.display()
            );
            Vec::new()
        }
    }
}

/// Write the registry atomically (tmp file + rename), creating parent
/// directories as needed.
pub fn save_registry(path: &Path, targets: &[RemoteTarget]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create registry dir {}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(targets).context("serialize remote registry")?;
    let tmp = tmp_sibling(path);
    fs::write(&tmp, json.as_bytes())
        .with_context(|| format!("write registry tmp file {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("publish registry file {}", path.display()))?;
    Ok(())
}

/// Sibling temp path unique to this process so concurrent writers do not
/// clobber each other's staging file.
fn tmp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sessions.json".to_string());
    path.with_file_name(format!("{name}.{}.tmp", std::process::id()))
}

/// Identity of a target: same host + user + port + zellij session means
/// the same remote destination.
fn same_identity(a: &RemoteTarget, b: &RemoteTarget) -> bool {
    a.host == b.host && a.user == b.user && a.port == b.port && a.session_name == b.session_name
}

/// Replace the target with the same identity, or append when new.
pub fn upsert_target(list: &mut Vec<RemoteTarget>, t: &RemoteTarget) {
    match list.iter().position(|e| same_identity(e, t)) {
        Some(i) => list[i] = t.clone(),
        None => list.push(t.clone()),
    }
}

/// Find a remembered target by host and zellij session name.
pub fn find_target<'a>(
    list: &'a [RemoteTarget],
    host: &str,
    session: &str,
) -> Option<&'a RemoteTarget> {
    list.iter()
        .find(|t| t.host == host && t.session_name == session)
}

/// Turn a free-form label into a valid zellij session name: ASCII
/// lowercase, digits and `-` separators only (zellij rejects anything
/// else). Falls back to `term-rust` when nothing usable remains.
pub fn suggest_session_name(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut pending_sep = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            pending_sep = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_sep = true;
        }
    }
    if out.is_empty() {
        "term-rust".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_registry_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "zt-remote-registry-{tag}-{}-{n}.json",
            std::process::id()
        ))
    }

    fn target(
        label: &str,
        host: &str,
        user: Option<&str>,
        port: Option<u16>,
        session: &str,
    ) -> RemoteTarget {
        RemoteTarget {
            label: label.to_string(),
            host: host.to_string(),
            user: user.map(|u| u.to_string()),
            port,
            session_name: session.to_string(),
        }
    }

    #[test]
    fn default_registry_path_shape() {
        let path = default_registry_path();
        assert!(path.ends_with("terminator-rust/sessions.json"), "{path:?}");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let path = temp_registry_path("roundtrip");
        let targets = vec![
            target(
                "Work box",
                "build.example.net",
                Some("alice"),
                Some(2222),
                "work",
            ),
            target("Bare", "10.0.0.9", None, None, "misc"),
        ];
        save_registry(&path, &targets).expect("save registry");
        let loaded = load_registry(&path);
        assert_eq!(loaded, targets);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_creates_missing_parent_dirs() {
        let dir =
            std::env::temp_dir().join(format!("zt-remote-registry-dirs-{}", std::process::id()));
        let path = dir.join("nested").join("sessions.json");
        save_registry(&path, &[target("T", "h", None, None, "s")]).expect("save registry");
        assert!(path.is_file());
        assert_eq!(load_registry(&path).len(), 1);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(dir.join("nested"));
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn missing_file_loads_empty() {
        let path = temp_registry_path("missing");
        let _ = fs::remove_file(&path);
        assert!(load_registry(&path).is_empty());
    }

    #[test]
    fn malformed_json_loads_empty() {
        let path = temp_registry_path("malformed");
        fs::write(&path, b"{ this is not json").expect("write malformed");
        assert!(load_registry(&path).is_empty());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn upsert_replaces_by_identity_and_appends_new() {
        let mut list = vec![target("Prod", "h1", Some("bob"), None, "main")];
        // Same identity, new label: replaced in place, not duplicated.
        upsert_target(
            &mut list,
            &target("Prod relabeled", "h1", Some("bob"), None, "main"),
        );
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "Prod relabeled");
        // Different port: distinct identity, appended.
        upsert_target(
            &mut list,
            &target("Prod alt port", "h1", Some("bob"), Some(2222), "main"),
        );
        assert_eq!(list.len(), 2);
        // Different session: distinct identity, appended.
        upsert_target(
            &mut list,
            &target("Other session", "h1", Some("bob"), None, "side"),
        );
        assert_eq!(list.len(), 3);
        // Different user: distinct identity, appended.
        upsert_target(
            &mut list,
            &target("Other user", "h1", Some("carol"), None, "main"),
        );
        assert_eq!(list.len(), 4);
        // Same identity again: still four, first entry updated.
        upsert_target(&mut list, &target("Again", "h1", Some("bob"), None, "main"));
        assert_eq!(list.len(), 4);
        assert_eq!(list[0].label, "Again");
    }

    #[test]
    fn find_target_matches_host_and_session() {
        let list = vec![
            target("A", "h1", Some("bob"), None, "main"),
            target("B", "h2", None, Some(22), "main"),
        ];
        let found = find_target(&list, "h2", "main");
        assert!(found.is_some());
        assert_eq!(found.map(|t| t.label.as_str()), Some("B"));
        assert!(find_target(&list, "h1", "other").is_none());
        assert!(find_target(&list, "hX", "main").is_none());
    }

    #[test]
    fn suggest_session_name_sanitizes() {
        assert_eq!(suggest_session_name("My Server! 2"), "my-server-2");
        assert_eq!(suggest_session_name("Build Box"), "build-box");
        assert_eq!(
            suggest_session_name("  leading and trailing  "),
            "leading-and-trailing"
        );
        assert_eq!(suggest_session_name("UPPER_case"), "upper-case");
        assert_eq!(suggest_session_name("host#42"), "host-42");
        assert_eq!(suggest_session_name("dot.name/path"), "dot-name-path");
        // Everything stripped: deterministic fallback.
        assert_eq!(suggest_session_name(""), "term-rust");
        assert_eq!(suggest_session_name("!!!"), "term-rust");
        assert_eq!(suggest_session_name("____"), "term-rust");
    }
}
