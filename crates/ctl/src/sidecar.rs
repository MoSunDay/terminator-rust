//! `oc-links.json`: pane-name -> pinned (db, session) sidecar persisted next
//! to the app's state.json. Keyed by pane manual title because pane ids are
//! re-allocated per app run; names are the stable addressing key.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// Absolute path of the opencoder per-workdir store.
    pub db: String,
    /// Pinned session id; None = resolve to the store's latest at use time.
    pub session: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Links {
    pub links: BTreeMap<String, Link>,
}

/// Sidecar path: `$XDG_CONFIG_HOME|$HOME/.config`/terminator-rust/oc-links.json
/// (same base the app uses for state.json).
pub fn sidecar_path() -> PathBuf {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".config"),
            _ => PathBuf::from(".config"),
        },
    };
    base.join("terminator-rust").join("oc-links.json")
}

/// Load the sidecar; a missing file is an empty map, a corrupt one is an
/// error (names are addressing keys -- silently wiping them is worse).
pub fn load(path: &std::path::Path) -> Result<Links> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Links::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

/// Load, set one pane's link and save atomically (tmp + rename).
pub fn set(path: &std::path::Path, pane: &str, link: Link) -> Result<()> {
    let mut all = load(path)?;
    all.links.insert(pane.to_string(), link);
    save(path, &all)
}

/// Load and drop one pane's link (no error when absent).
pub fn remove(path: &std::path::Path, pane: &str) -> Result<()> {
    let mut all = load(path)?;
    if all.links.remove(pane).is_some() {
        save(path, &all)?;
    }
    Ok(())
}

fn save(path: &std::path::Path, links: &Links) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(links).context("serialize links")?;
    std::fs::write(&tmp, text).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("install {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("oc-links-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn missing_file_is_empty_and_roundtrip_works() {
        let p = tmp("roundtrip");
        assert_eq!(load(&p).unwrap(), Links::default());
        set(
            &p,
            "agent1",
            Link {
                db: "/x/opencoder.db".into(),
                session: Some("01M".into()),
            },
        )
        .unwrap();
        set(
            &p,
            "agent2",
            Link {
                db: "/y/opencoder.db".into(),
                session: None,
            },
        )
        .unwrap();
        let all = load(&p).unwrap();
        assert_eq!(all.links.len(), 2);
        assert_eq!(all.links["agent1"].session.as_deref(), Some("01M"));
        remove(&p, "agent1").unwrap();
        assert!(!load(&p).unwrap().links.contains_key("agent1"));
        remove(&p, "ghost").unwrap(); // absent is fine
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let p = tmp("corrupt");
        std::fs::write(&p, "{not json").unwrap();
        assert!(load(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }
}
