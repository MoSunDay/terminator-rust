//! Desktop launches can inherit a stale SHELL from their launcher on macOS.

pub(crate) fn default_shell() -> String {
    #[cfg(target_os = "macos")]
    if let Some(shell) = account_shell() {
        return shell;
    }
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

#[cfg(target_os = "macos")]
fn account_shell() -> Option<String> {
    let mut buffer = vec![0u8; 16384];
    loop {
        // getpwuid_r stores strings in our buffer; copy the shell before it drops.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && buffer.len() < 1_048_576 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || result.is_null() || entry.pw_shell.is_null() {
            return None;
        }
        let shell = unsafe { std::ffi::CStr::from_ptr(entry.pw_shell) }
            .to_str()
            .ok()?;
        return (!shell.is_empty()).then(|| shell.to_string());
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn local_pane_uses_account_shell_and_exports_it() {
        let expected = super::account_shell().expect("current account shell");
        let options = crate::SessionOpts::local_shell(80, 24);
        assert_eq!(options.argv, vec![expected.clone(), "-i".to_string()]);
        assert!(options.env.contains(&format!("SHELL={expected}")));
    }
}
