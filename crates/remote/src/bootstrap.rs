//! Remote shell command and SSH arguments for named PTY sessions.

use crate::escape::sh_single_quote;
use crate::registry::RemoteTarget;

/// The remote host has no `terminator-session`; the app opens plain SSH.
pub const EXIT_NO_KEEPER_SHELL: i32 = 42;

/// Attach to the named remote session, creating it if needed.
/// The keeper owns the PTY independently of this SSH connection.
pub fn bootstrap_command(target: &RemoteTarget) -> String {
    let name = sh_single_quote(&target.session_name);
    let title = sh_single_quote(if target.label.is_empty() {
        &target.session_name
    } else {
        &target.label
    });
    format!(
        "command -v terminator-session >/dev/null 2>&1 || exit {EXIT_NO_KEEPER_SHELL}\nexec terminator-session attach {name} --title {title}\n"
    )
}

pub fn remote_probe_command() -> &'static str {
    "command -v terminator-session || echo NO_SESSION_KEEPER"
}

/// SSH forces a PTY and detects connections that have stopped responding.
pub fn ssh_argv(target: &RemoteTarget, remote_cmd: Option<&str>) -> Vec<String> {
    let mut argv = vec![
        "ssh".to_string(),
        "-tt".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=15".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
    ];
    if let Some(port) = target.port {
        argv.push("-p".to_string());
        argv.push(port.to_string());
    }
    argv.push(match &target.user {
        Some(user) if !user.is_empty() => format!("{user}@{}", target.host),
        _ => target.host.clone(),
    });
    if let Some(cmd) = remote_cmd {
        argv.push(cmd.to_string());
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(session: &str) -> RemoteTarget {
        RemoteTarget {
            label: "release build".into(),
            host: "box.example.net".into(),
            user: Some("alice".into()),
            port: Some(2222),
            session_name: session.into(),
        }
    }

    #[test]
    fn bootstrap_uses_only_own_keeper() {
        let cmd = bootstrap_command(&target("work"));
        assert_eq!(cmd, "command -v terminator-session >/dev/null 2>&1 || exit 42\nexec terminator-session attach 'work' --title 'release build'\n");
    }

    #[test]
    fn bootstrap_quotes_name_and_title() {
        let mut t = target("w';echo bad");
        t.label = "it's working".into();
        let cmd = bootstrap_command(&t);
        assert!(cmd.contains("attach 'w'\\'';echo bad' --title 'it'\\''s working'"));
    }

    #[test]
    fn ssh_arguments_keep_remote_command_intact() {
        let argv = ssh_argv(&target("work"), Some("echo hi"));
        assert_eq!(argv.first().map(String::as_str), Some("ssh"));
        assert_eq!(argv.last().map(String::as_str), Some("echo hi"));
        assert!(argv.contains(&"alice@box.example.net".to_string()));
        assert!(argv.contains(&"2222".to_string()));
    }
}
