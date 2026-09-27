//! Invisible remote sessions over ssh.
//!
//! Remote panes use `terminator-session` on the host to preserve their
//! shell PTY, with plain SSH when the keeper is unavailable.
//!
//! Sessions are remembered in a JSON registry so a dead pane can be
//! reconnected to the same named remote session with its state intact.
//!
//! Pure data structs and free functions only; all builders return plain
//! values (`String` / `Vec<String>`) and never touch the network or the
//! filesystem outside the registry helpers.

pub mod bootstrap;
pub mod escape;
pub mod registry;
pub mod session;

pub use bootstrap::{bootstrap_command, remote_probe_command, ssh_argv, EXIT_NO_KEEPER_SHELL};
pub use escape::{contains_shell_metachar, heredoc_literal, sh_quote_join, sh_single_quote};
pub use registry::{
    default_registry_path, find_target, load_registry, save_registry, suggest_session_name,
    upsert_target, RemoteTarget,
};
pub use session::{
    interpret_exit, is_disconnect, local_plan, reconnect_argv, remote_plan, PaneKind, PaneStatus,
    SpawnPlan, EXIT_NO_KEEPER, EXIT_SSH_FAIL,
};
