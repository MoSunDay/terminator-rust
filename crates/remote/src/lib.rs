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

/// 50/50 mix of two `#rrggbb` colors; returns `b` unchanged when either
/// side fails to parse.
pub fn mix_hex(a: &str, b: &str) -> String {
    match (hex_rgb(a), hex_rgb(b)) {
        (Some((r1, g1, b1)), Some((r2, g2, b2))) => {
            format!(
                "#{:02x}{:02x}{:02x}",
                (r1 + r2) / 2,
                (g1 + g2) / 2,
                (b1 + b2) / 2
            )
        }
        _ => b.to_string(),
    }
}

fn hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    Some((v(0)?, v(2)?, v(4)?))
}

#[cfg(test)]
mod tests {
    use super::mix_hex;

    #[test]
    fn mix_hex_averages_and_falls_back() {
        assert_eq!(mix_hex("#000000", "#ffffff"), "#7f7f7f");
        assert_eq!(mix_hex("bad", "#112233"), "#112233");
        assert_eq!(mix_hex("#112233", "nope"), "nope");
    }
}
