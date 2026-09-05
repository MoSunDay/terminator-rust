//! Remote-side bootstrap: one idempotent shell command that installs a
//! minimal zellij config + layout on the remote host and then execs
//! `zellij attach --create`, so zellij is completely invisible (no tab
//! bar, no status bar, no pane frames, no welcome screen, no hints).

use crate::escape::{heredoc_literal, sh_single_quote};
use crate::registry::RemoteTarget;

/// Exit code the remote command uses to signal "no zellij installed";
/// the local side degrades to a plain ssh shell on receipt.
pub const EXIT_NO_ZELLIJ_SHELL: i32 = 42;

/// Official zellij "no plugins at all" layout: a single pane, nothing else.
pub const ZT_MINI_KDL: &str = "layout {\n    pane\n}\n";

/// Neutral fallback palette (fg, bg, then red..orange) used when no theme
/// palette is available, e.g. plain reconnects.
pub const DEFAULT_PALETTE_HEX: [&str; 9] = [
    "#c0c0c0", "#1e1e1e", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd",
    "#f14c4c",
];

/// Heredoc tag for all remote config writes. None of the generated file
/// bodies ever contain this line (asserted in tests).
const HD_TAG: &str = "EOF";

/// Remote layout file contents: a bare pane, no tab-bar or status plugins.
pub fn zt_mini_kdl() -> &'static str {
    ZT_MINI_KDL
}

/// Remote config contents. `fg`/`bg`/`c1..c7` are hex colors like
/// `"#1e1e2e"`; the `zt` theme is flat so zellij draws nothing of its own
/// on top of the terminal colors, pane frames are off, and all default
/// keybinds are cleared so no chord is intercepted.
///
/// The nine parameters mirror the 9-slot palette (fg, bg, c1..c7).
#[allow(clippy::too_many_arguments)] // palette slots, see doc
pub fn zt_config_kdl(
    fg: &str,
    bg: &str,
    c1: &str,
    c2: &str,
    c3: &str,
    c4: &str,
    c5: &str,
    c6: &str,
    c7: &str,
) -> String {
    format!(
        concat!(
            "pane_frames false\n",
            "auto_layout false\n",
            "default_layout \"zt-mini\"\n",
            "session_serialization true\n",
            "keybinds clear-defaults=true {{\n",
            "}}\n",
            "theme \"zt\"\n",
            "themes {{\n",
            "    zt {{\n",
            "        fg \"{fg}\"\n",
            "        bg \"{bg}\"\n",
            "        red \"{c1}\"\n",
            "        green \"{c2}\"\n",
            "        yellow \"{c3}\"\n",
            "        blue \"{c4}\"\n",
            "        magenta \"{c5}\"\n",
            "        cyan \"{c6}\"\n",
            "        orange \"{c7}\"\n",
            "        black \"{bg}\"\n",
            "        white \"{fg}\"\n",
            "        bright_black \"{bb}\"\n",
            "        bright_red \"{c1}\"\n",
            "        bright_green \"{c2}\"\n",
            "        bright_yellow \"{c3}\"\n",
            "        bright_blue \"{c4}\"\n",
            "        bright_magenta \"{c5}\"\n",
            "        bright_cyan \"{c6}\"\n",
            "        bright_white \"{fg}\"\n",
            "    }}\n",
            "}}\n"
        ),
        bb = crate::mix_hex(fg, bg),
        fg = fg,
        bg = bg,
        c1 = c1,
        c2 = c2,
        c3 = c3,
        c4 = c4,
        c5 = c5,
        c6 = c6,
        c7 = c7,
    )
}

/// The single remote `sh -c` body:
///
/// 1. bail out with [`EXIT_NO_ZELLIJ_SHELL`] when zellij is missing;
/// 2. `mkdir -p` the config dir;
/// 3. idempotently (re)write `config.kdl`, `layouts/zt-mini.kdl` and
///    `zt-mini.kdl` via quoted heredocs;
/// 4. `exec zellij --config-dir ~/.cache/zt attach --create <session>`.
///
/// Every interpolated value is either heredoc-quoted (file bodies,
/// palette) or single-quoted (session name); `$HOME` paths are our own
/// literals inside double quotes. Running it twice is a no-op change.
pub fn bootstrap_command(target: &RemoteTarget, palette_hex: [&str; 9]) -> String {
    let [fg, bg, c1, c2, c3, c4, c5, c6, c7] = palette_hex;
    let config_kdl = zt_config_kdl(fg, bg, c1, c2, c3, c4, c5, c6, c7);
    let layout_kdl = zt_mini_kdl();
    let config_hd = heredoc_literal(HD_TAG, &config_kdl);
    let layout_hd = heredoc_literal(HD_TAG, layout_kdl);

    let mut cmd = String::with_capacity(config_kdl.len() + layout_kdl.len() * 2 + 256);
    cmd.push_str("command -v zellij >/dev/null 2>&1 || exit ");
    cmd.push_str(&EXIT_NO_ZELLIJ_SHELL.to_string());
    cmd.push('\n');
    cmd.push_str("mkdir -p \"$HOME/.cache/zt/layouts\"\n");
    cmd.push_str(&format!(
        "cat > \"$HOME/.cache/zt/config.kdl\" {config_hd}\n"
    ));
    cmd.push_str(&format!(
        "cat > \"$HOME/.cache/zt/layouts/zt-mini.kdl\" {layout_hd}\n"
    ));
    cmd.push_str(&format!(
        "cat > \"$HOME/.cache/zt/zt-mini.kdl\" {layout_hd}\n"
    ));
    cmd.push_str(&format!(
        "exec zellij --config-dir \"$HOME/.cache/zt\" attach --create {}\n",
        sh_single_quote(&target.session_name)
    ));
    cmd
}

/// Pre-flight probe: prints the zellij version, or `NO_ZELLIJ`.
pub fn remote_probe_command() -> &'static str {
    "command -v zellij && zellij --version || echo NO_ZELLIJ"
}

/// ssh argv for `target`, optionally carrying a remote command as the
/// final argument. Forces tty allocation and keeps half-dead connections
/// from wedging the pane. Returns plain argv; the caller spawns it
/// directly (no shell on the local side).
pub fn ssh_argv(target: &RemoteTarget, remote_cmd: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "ssh".to_string(),
        "-tt".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=15".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
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
            label: "Test".to_string(),
            host: "box.example.net".to_string(),
            user: Some("alice".to_string()),
            port: Some(2222),
            session_name: session.to_string(),
        }
    }

    fn palette() -> [&'static str; 9] {
        [
            "#c0c0c0", "#1e1e1e", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff",
            "#ff8000",
        ]
    }

    #[test]
    fn mini_layout_has_no_plugins_or_bars() {
        let kdl = zt_mini_kdl();
        assert_eq!(kdl, "layout {\n    pane\n}\n");
        for banned in [
            "plugin",
            "tab-bar",
            "status-bar",
            "pane_frames",
            "welcome",
            "hint",
        ] {
            assert!(
                !kdl.to_lowercase().contains(banned),
                "layout leaked {banned}"
            );
        }
    }

    #[test]
    fn config_kdl_shape_and_palette() {
        let p = palette();
        let kdl = zt_config_kdl(p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8]);
        for expected in [
            "pane_frames false\n",
            "auto_layout false\n",
            "default_layout \"zt-mini\"\n",
            "session_serialization true\n",
            "keybinds clear-defaults=true {\n}\n",
            "theme \"zt\"\n",
        ] {
            assert!(
                kdl.contains(expected),
                "config missing {expected:?} in:\n{kdl}"
            );
        }
        for hex in p {
            assert!(kdl.contains(&format!("\"{hex}\"")), "palette {hex} missing");
        }
        // Heredoc safety: no generated body may contain the terminator line.
        assert!(!kdl.lines().any(|l| l == "EOF"));
        assert!(!zt_mini_kdl().lines().any(|l| l == "EOF"));
    }

    #[test]
    fn bright_black_is_a_mix_not_plain_bg() {
        let p = palette();
        let kdl = zt_config_kdl(p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8]);
        // SGR-90 text must stay visible: bright_black is a bg/fg mix, not bg.
        assert!(kdl.contains(&format!("bright_black \"{}\"", crate::mix_hex(p[0], p[1]))));
        assert!(!kdl.contains(&format!("bright_black \"{}\"", p[1])));
    }

    #[test]
    fn bootstrap_command_structure() {
        let cmd = bootstrap_command(&target("work"), palette());
        // Degrade marker first.
        assert!(
            cmd.starts_with("command -v zellij >/dev/null 2>&1 || exit 42\n"),
            "{cmd}"
        );
        // Idempotent heredoc writes: each of the three files exactly once,
        // always via a quoted heredoc.
        for file in [
            "$HOME/.cache/zt/config.kdl",
            "$HOME/.cache/zt/layouts/zt-mini.kdl",
            "$HOME/.cache/zt/zt-mini.kdl",
        ] {
            let needle = format!("cat > \"{file}\" <<'EOF'");
            assert_eq!(
                cmd.matches(&needle).count(),
                1,
                "expected one write of {file}"
            );
        }
        // Heredoc terminators balance: three opens, three closers.
        assert_eq!(cmd.matches("<<'EOF'").count(), 3);
        assert_eq!(cmd.lines().filter(|l| *l == "EOF").count(), 3);
        // Config dir created before use, exec attaches with config dir.
        assert!(cmd.contains("mkdir -p \"$HOME/.cache/zt/layouts\"\n"));
        assert!(
            cmd.contains("exec zellij --config-dir \"$HOME/.cache/zt\" attach --create 'work'\n")
        );
        assert!(cmd.ends_with("attach --create 'work'\n"));
    }

    #[test]
    fn bootstrap_command_quotes_session_name_everywhere() {
        // Metachar-laden name without single quotes: the raw name may only
        // occur inside the single-quoted form.
        let nasty = "w;rm -rf `echo $HOME`";
        let cmd = bootstrap_command(&target(nasty), palette());
        let quoted = sh_single_quote(nasty);
        assert!(cmd.matches(&quoted).count() >= 1);
        assert_eq!(cmd.matches(nasty).count(), cmd.matches(&quoted).count());
        assert!(cmd.contains(&format!("attach --create {quoted}\n")));

        // With an embedded single quote the raw name must not occur at all:
        // it only exists in escaped form.
        let nastier = "w';rm -rf /tmp/x";
        let cmd2 = bootstrap_command(&target(nastier), palette());
        let quoted2 = sh_single_quote(nastier);
        assert_eq!(cmd2.matches(nastier).count(), 0);
        assert!(cmd2.matches(&quoted2).count() >= 1);
        assert!(cmd2.contains(&format!("attach --create {quoted2}\n")));

        // Layout/config bodies still travel verbatim, heredoc-delimited.
        assert!(cmd.contains(zt_mini_kdl()));
    }

    #[test]
    fn bootstrap_command_is_stable_and_idempotent_by_construction() {
        let t = target("work");
        let a = bootstrap_command(&t, palette());
        let b = bootstrap_command(&t, palette());
        assert_eq!(a, b);
        // Same session + palette: each file body appears exactly once, so
        // re-running only ever rewrites identical bytes.
        assert_eq!(cmd_count(&a, &zt_config_kdl_from_palette()), 1);
        assert_eq!(a.matches(zt_mini_kdl()).count(), 2); // two layout files
    }

    fn zt_config_kdl_from_palette() -> String {
        let p = palette();
        zt_config_kdl(p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8])
    }

    fn cmd_count(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    #[test]
    fn probe_command_shape() {
        assert_eq!(
            remote_probe_command(),
            "command -v zellij && zellij --version || echo NO_ZELLIJ"
        );
    }

    #[test]
    fn ssh_argv_full_shape() {
        let argv = ssh_argv(&target("work"), Some("echo hi"));
        assert_eq!(
            argv,
            vec![
                "ssh",
                "-tt",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-p",
                "2222",
                "alice@box.example.net",
                "echo hi",
            ]
        );
    }

    #[test]
    fn ssh_argv_minimal_shape() {
        let t = RemoteTarget {
            label: "Bare".to_string(),
            host: "h".to_string(),
            user: None,
            port: None,
            session_name: "s".to_string(),
        };
        assert_eq!(
            ssh_argv(&t, None),
            vec![
                "ssh",
                "-tt",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "h",
            ]
        );
        // Remote command lands as the final argv element, unmodified.
        let with_cmd = ssh_argv(&t, Some("cmd 'q' $x"));
        assert_eq!(*with_cmd.last().expect("last"), "cmd 'q' $x");
    }

    #[test]
    fn ssh_argv_empty_user_means_bare_host() {
        let t = RemoteTarget {
            label: "L".to_string(),
            host: "h2".to_string(),
            user: Some(String::new()),
            port: None,
            session_name: "s".to_string(),
        };
        assert_eq!(ssh_argv(&t, None)[ssh_argv(&t, None).len() - 1], "h2");
    }
}
