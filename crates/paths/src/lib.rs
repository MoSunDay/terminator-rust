//! Shared filesystem locations for terminator-rust.
//!
//! The config root is `$HOME/.terminator-rust` (dotfile style, no XDG
//! base-dir indirection). Releases before 2026-09 kept everything under
//! `$XDG_CONFIG_HOME|$HOME/.config`/terminator-rust; [`migrate_legacy`]
//! copies a legacy file forward (best-effort, an existing new file wins)
//! so upgrades keep their state.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Config root: `$HOME/.terminator-rust`, or a relative
/// `.terminator-rust` when HOME is unset/empty (same fallback shape the
/// pre-2026-09 code used).
pub fn config_dir() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h).join(".terminator-rust"),
        _ => PathBuf::from(".terminator-rust"),
    }
}

/// Runtime dir: `$XDG_RUNTIME_DIR/terminator-rust` (per-session tmpfs),
/// falling back to [`config_dir`] when XDG_RUNTIME_DIR is unset/empty
/// (same fallback shape the control socket always used). Created
/// best-effort (`log::warn` on failure); the path is returned regardless
/// so callers can treat a bind failure as the real error.
pub fn runtime_dir() -> PathBuf {
    let dir = runtime_dir_from(std::env::var_os("XDG_RUNTIME_DIR"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("runtime dir: create {}: {e}", dir.display());
    }
    dir
}

/// Pure core of [`runtime_dir`]: `xdg/terminator-rust` when `xdg` is set
/// and non-empty, else the config root. Split out so tests (and callers
/// that already hold the env value) stay env-free.
pub fn runtime_dir_from(xdg: Option<OsString>) -> PathBuf {
    match xdg {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("terminator-rust"),
        _ => config_dir(),
    }
}

/// Resolve `name` under the current config root, migrating a legacy file
/// forward on first use. Always returns `config_dir().join(name)`.
pub fn migrate_legacy(name: &str) -> PathBuf {
    migrate_into(&config_dir(), &legacy_config_dir(), name)
}

/// Legacy config root (pre-2026-09): `$XDG_CONFIG_HOME|$HOME/.config`
/// plus `terminator-rust`. Read-only, only for migration reads.
fn legacy_config_dir() -> PathBuf {
    legacy_base(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
    .join("terminator-rust")
}

/// Pure core of the legacy base resolution: `xdg` when non-empty, else
/// `home/.config`, else a relative `.config`.
fn legacy_base(xdg: Option<&OsStr>, home: Option<&OsStr>) -> PathBuf {
    match xdg {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => match home {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".config"),
            _ => PathBuf::from(".config"),
        },
    }
}

/// Copy `dir_old/name` to `dir_new/name` when the new file is absent and
/// the legacy one is a regular file. Always returns `dir_new/name`;
/// create/copy failures are logged, never propagated.
fn migrate_into(dir_new: &Path, dir_old: &Path, name: &str) -> PathBuf {
    let target = dir_new.join(name);
    if target.is_file() || !dir_old.join(name).is_file() {
        return target;
    }
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!(
                    "migrate legacy config: create {}: {e}; using {}",
                    parent.display(),
                    target.display()
                );
                return target;
            }
        }
    }
    if let Err(e) = std::fs::copy(dir_old.join(name), &target) {
        log::warn!(
            "migrate legacy config: {} -> {}: {e}",
            dir_old.join(name).display(),
            target.display()
        );
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Unique scratch dir per test (temp dir + process id + counter),
    /// cleaned up best-effort by the caller.
    fn scratch(tag: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "terminator-paths-test-{}-{}-{n}",
            std::process::id(),
            tag
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dirs: &[PathBuf]) {
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn legacy_file_copied_when_new_missing() {
        let old = scratch("old-copy");
        let root = scratch("new-copy");
        let new = root.join("deep"); // parent dirs must be created
        std::fs::write(old.join("sessions.json"), b"legacy").unwrap();
        let got = migrate_into(&new, &old, "sessions.json");
        assert_eq!(got, new.join("sessions.json"));
        assert_eq!(std::fs::read(&got).unwrap(), b"legacy");
        cleanup(&[old, root]);
    }

    #[test]
    fn existing_new_file_not_overwritten() {
        let old = scratch("old-wins");
        let new = scratch("new-wins");
        std::fs::write(old.join("state.json"), b"legacy").unwrap();
        std::fs::write(new.join("state.json"), b"current").unwrap();
        let got = migrate_into(&new, &old, "state.json");
        assert_eq!(got, new.join("state.json"));
        assert_eq!(std::fs::read(&got).unwrap(), b"current");
        cleanup(&[old, new]);
    }

    #[test]
    fn no_legacy_file_returns_new_path_uncreated() {
        let old = scratch("old-none");
        let new = scratch("new-none");
        let got = migrate_into(&new, &old, "oc-links.json");
        assert_eq!(got, new.join("oc-links.json"));
        assert!(!got.exists());
        // Nothing was created inside the new dir.
        assert!(std::fs::read_dir(&new).unwrap().next().is_none());
        cleanup(&[old, new]);
    }

    #[test]
    fn legacy_base_resolution() {
        assert_eq!(
            legacy_base(Some(OsStr::new("/xdg")), Some(OsStr::new("/home/u"))),
            PathBuf::from("/xdg")
        );
        assert_eq!(
            legacy_base(None, Some(OsStr::new("/home/u"))),
            PathBuf::from("/home/u/.config")
        );
        assert_eq!(
            legacy_base(Some(OsStr::new("")), Some(OsStr::new("/home/u"))),
            PathBuf::from("/home/u/.config")
        );
        assert_eq!(legacy_base(None, None), PathBuf::from(".config"));
        assert_eq!(
            legacy_base(None, Some(OsStr::new(""))),
            PathBuf::from(".config")
        );
    }

    #[test]
    fn config_dir_lives_under_home() {
        // Shape check only (HOME is always set in CI); the relative
        // fallback is covered by `legacy_base_resolution`'s pattern.
        let dir = config_dir();
        assert!(
            dir == Path::new(".terminator-rust") || dir.ends_with(".terminator-rust"),
            "{dir:?}"
        );
    }

    #[test]
    fn runtime_dir_from_prefers_xdg_then_config_root() {
        assert_eq!(
            runtime_dir_from(Some(OsString::from("/run/user/1000"))),
            PathBuf::from("/run/user/1000/terminator-rust")
        );
        assert_eq!(runtime_dir_from(None), config_dir());
        assert_eq!(runtime_dir_from(Some(OsString::new())), config_dir());
    }

    #[test]
    fn runtime_dir_lives_under_terminator_rust() {
        // Both branches end in the crate dir: `terminator-rust` under
        // $XDG_RUNTIME_DIR, `.terminator-rust` (the config root) without.
        // Env-free on purpose - asserting the ambient `runtime_dir()`
        // failed whenever XDG_RUNTIME_DIR was unset, because
        // Path::ends_with compares whole components.
        assert!(runtime_dir_from(Some(OsString::from("/run/u/9"))).ends_with("terminator-rust"));
        assert!(runtime_dir_from(None).ends_with(".terminator-rust"));
    }
}
