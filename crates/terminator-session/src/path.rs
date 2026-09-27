use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("session name must contain 1..80 lowercase letters, digits or hyphens");
    }
    Ok(())
}

pub fn session_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is unset")?;
    let dir = PathBuf::from(home).join(".terminator-rust/sessions");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

pub fn socket_path(name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    Ok(session_dir()?.join(format!("{name}.sock")))
}

#[cfg(test)]
mod tests {
    use super::validate_name;

    #[test]
    fn names_cannot_escape_socket_dir() {
        for name in ["", "../x", "Foo", "a_b", "a/b", "a b"] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        assert!(validate_name("project-42").is_ok());
    }
}
