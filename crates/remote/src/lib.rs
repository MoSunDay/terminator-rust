//! Invisible zellij remote sessions over ssh.
//!
//! Remote panes are spawned as `ssh -tt` sessions that attach to (or
//! create) a zellij session on the remote host using a generated minimal
//! config, so zellij stays completely invisible: no tab bar, no status
//! bar, no pane frames, no welcome screen. If zellij is missing on the
//! remote host the session degrades to a plain ssh shell.
//!
//! Sessions are remembered in a JSON registry so a dead pane can be
//! reconnected to the same zellij session with its state intact.
//!
//! Pure data structs and free functions only; all builders return plain
//! values (`String` / `Vec<String>`) and never touch the network or the
//! filesystem outside the registry helpers.

pub mod bootstrap;
pub mod escape;
pub mod registry;
pub mod session;

pub use bootstrap::{
    bootstrap_command, remote_probe_command, ssh_argv, zt_config_kdl, zt_mini_kdl,
    DEFAULT_PALETTE_HEX, EXIT_NO_ZELLIJ_SHELL, ZT_MINI_KDL,
};
pub use escape::{contains_shell_metachar, heredoc_literal, sh_quote_join, sh_single_quote};
pub use registry::{
    default_registry_path, find_target, load_registry, save_registry, suggest_session_name,
    upsert_target, RemoteTarget,
};
pub use session::{
    interpret_exit, local_plan, reconnect_argv, remote_plan, PaneKind, PaneStatus, SpawnPlan,
    EXIT_NO_ZELLIJ,
};
