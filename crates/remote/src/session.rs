//! Spawn plans and exit-code interpretation for local and remote panes.
//!
//! Builders return plain `SpawnPlan` values; the pane/PTY layer turns
//! them into a spawned child. Nothing here performs I/O.

use crate::bootstrap::{bootstrap_command, ssh_argv, DEFAULT_PALETTE_HEX};
use crate::registry::RemoteTarget;

/// Remote exit code meaning "zellij is not installed on that host" (see
/// [`crate::bootstrap::EXIT_NO_ZELLIJ_SHELL`]).
pub const EXIT_NO_ZELLIJ: i32 = 42;

/// What a pane is running.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneKind {
    /// A local interactive shell.
    Local,
    /// An ssh session to a remote zellij (or plain shell after degrade).
    Remote(RemoteTarget),
}

/// A fully-resolved child process description: argv plus extra
/// environment entries (`"K=V"` strings) layered over the parent env.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPlan {
    pub argv: Vec<String>,
    pub env: Vec<String>,
}

/// Lifecycle status of a pane's child process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneStatus {
    /// No exit observed yet.
    Running,
    /// Child exited with the given code.
    Exited(i32),
    /// Remote reported "no zellij installed" (exit 42): caller should
    /// degrade to a plain ssh shell.
    NoZellij,
    /// Reader hit EOF without an exit code (connection drop); set by the
    /// UI layer when that happens.
    Disconnected,
}

/// Interactive local shell: `$SHELL -i`, falling back to `/bin/sh -i`.
pub fn local_plan() -> SpawnPlan {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string());
    SpawnPlan {
        argv: vec![shell, "-i".to_string()],
        env: Vec::new(),
    }
}

/// Remote pane spawn plan.
///
/// When `preflight_ok` is true zellij is assumed present and the argv
/// carries the idempotent bootstrap command (attach --create the
/// remembered session). Otherwise the argv is a plain `ssh -tt` shell and
/// the bootstrap is skipped entirely, degrading gracefully.
pub fn remote_plan(target: &RemoteTarget, palette_hex: [&str; 9], preflight_ok: bool) -> SpawnPlan {
    let remote_cmd = if preflight_ok {
        Some(bootstrap_command(target, palette_hex))
    } else {
        None
    };
    SpawnPlan {
        argv: ssh_argv(target, remote_cmd.as_deref()),
        env: vec![
            "TERM=xterm-256color".to_string(),
            "TERM_PROGRAM=terminator-rust".to_string(),
            "LC_ALL=C.UTF-8".to_string(),
        ],
    }
}

/// Reconnect to a remembered session: same idempotent bootstrap, so the
/// zellij session (and its state) is reattached or recreated.
pub fn reconnect_argv(target: &RemoteTarget) -> Vec<String> {
    ssh_argv(
        target,
        Some(&bootstrap_command(target, DEFAULT_PALETTE_HEX)),
    )
}

/// Map an observed wait-status exit code to a [`PaneStatus`].
///
/// `None` (still running) maps to [`PaneStatus::Running`]; `Some(42)` is
/// the remote "no zellij" marker; anything else is a plain exit.
pub fn interpret_exit(code: Option<i32>) -> PaneStatus {
    match code {
        None => PaneStatus::Running,
        Some(EXIT_NO_ZELLIJ) => PaneStatus::NoZellij,
        Some(other) => PaneStatus::Exited(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> RemoteTarget {
        RemoteTarget {
            label: "Work".to_string(),
            host: "box.example.net".to_string(),
            user: Some("alice".to_string()),
            port: Some(2222),
            session_name: "work".to_string(),
        }
    }

    fn palette() -> [&'static str; 9] {
        DEFAULT_PALETTE_HEX
    }

    #[test]
    fn local_plan_runs_login_shell_interactive() {
        let plan = local_plan();
        assert_eq!(plan.argv.len(), 2);
        assert_eq!(plan.argv[1], "-i");
        assert!(!plan.argv[0].is_empty());
        assert!(plan.env.is_empty());
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() {
                assert_eq!(plan.argv[0], shell);
            }
        }
    }

    #[test]
    fn remote_plan_with_preflight_carries_bootstrap() {
        let plan = remote_plan(&target(), palette(), true);
        assert_eq!(plan.argv.first().map(String::as_str), Some("ssh"));
        assert_eq!(
            plan.env,
            vec![
                "TERM=xterm-256color",
                "TERM_PROGRAM=terminator-rust",
                "LC_ALL=C.UTF-8",
            ]
        );
        let last = plan.argv.last().expect("remote cmd argv");
        assert!(last.starts_with("command -v zellij"));
        assert!(last.contains("attach --create 'work'"));
        assert!(last.contains("exit 42"));
        // user@host is present with the port.
        assert!(plan.argv.contains(&"alice@box.example.net".to_string()));
        assert!(plan.argv.contains(&"2222".to_string()));
    }

    #[test]
    fn remote_plan_without_preflight_is_plain_ssh() {
        let plan = remote_plan(&target(), palette(), false);
        assert_eq!(plan.argv.first().map(String::as_str), Some("ssh"));
        let last = plan.argv.last().expect("last argv");
        assert_eq!(last, "alice@box.example.net", "no remote command expected");
        // Env is still pinned for consistent rendering.
        assert_eq!(plan.env.len(), 3);
    }

    #[test]
    fn remote_plan_env_entries_are_kv() {
        for entry in remote_plan(&target(), palette(), true).env {
            assert_eq!(entry.match_indices('=').count(), 1, "entry {entry}");
            assert!(!entry.starts_with('=') && !entry.ends_with('='));
        }
    }

    #[test]
    fn reconnect_reuses_idempotent_bootstrap() {
        let argv = reconnect_argv(&target());
        assert_eq!(argv.first().map(String::as_str), Some("ssh"));
        let last = argv.last().expect("reconnect cmd");
        assert!(last.contains("attach --create 'work'"));
        assert!(last.contains("exit 42"));
        assert_eq!(last, &bootstrap_command(&target(), DEFAULT_PALETTE_HEX));
    }

    #[test]
    fn interpret_exit_maps_codes() {
        assert_eq!(interpret_exit(None), PaneStatus::Running);
        assert_eq!(interpret_exit(Some(EXIT_NO_ZELLIJ)), PaneStatus::NoZellij);
        assert_eq!(interpret_exit(Some(0)), PaneStatus::Exited(0));
        assert_eq!(interpret_exit(Some(1)), PaneStatus::Exited(1));
        assert_eq!(interpret_exit(Some(130)), PaneStatus::Exited(130));
        // Disconnected is never produced here: the UI sets it on EOF.
        assert_ne!(interpret_exit(None), PaneStatus::Disconnected);
    }
}
