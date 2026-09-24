//! Multi-instance argv parsing, split out of `args.rs` to keep both files
//! small: the global `--socket` flag and the `instances`/`migrate`
//! subcommands. Same contract as `args`: pure, no I/O, bad input is
//! `Err(String)`.

use ipc_proto::PaneSelector;

use crate::args::{unknown_flag, value_of, Cli};

/// Strip the leading global `--socket` tokens (both `--socket <path>` and
/// `--socket=<path>`; repeats keep the last value) and return the override
/// plus the remaining tokens. The flag is only valid BEFORE the subcommand,
/// so anything after it is left in place for the subcommand parser to
/// reject via [`unknown_flag`].
pub fn strip_global_socket(args: &[String]) -> Result<(Option<String>, &[String]), String> {
    let mut socket = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--socket" {
            socket = Some(
                args.get(i + 1)
                    .ok_or_else(|| "flag --socket requires a value".to_string())?
                    .clone(),
            );
            i += 2;
        } else if let Some(v) = a.strip_prefix("--socket=") {
            if v.is_empty() {
                return Err("flag --socket requires a value".to_string());
            }
            socket = Some(v.to_string());
            i += 1;
        } else {
            return Ok((socket, &args[i..]));
        }
    }
    Ok((socket, &[]))
}

/// `instances [--json] [--all]`: no positionals.
pub fn parse_instances(rest: &[String]) -> Result<Cli, String> {
    let mut json = false;
    let mut all = false;
    for a in rest {
        match a.as_str() {
            "--json" => json = true,
            "--all" => all = true,
            other => return Err(unknown_flag(other)),
        }
    }
    Ok(Cli::Instances { json, all })
}

/// `migrate <pane> --to <socket-path>`: `--to` is required and may sit on
/// either side of the pane positional.
pub fn parse_migrate(rest: &[String]) -> Result<Cli, String> {
    let mut pane: Option<PaneSelector> = None;
    let mut to: Option<String> = None;
    for i in crate::args::flag_walk(rest) {
        let a = rest[i].as_str();
        if a == "--to" {
            to = Some(value_of(rest, i, "--to")?);
        } else if a.starts_with("--") {
            return Err(unknown_flag(a));
        } else if pane.replace(crate::args::parse_pane(a)).is_some() {
            return Err(format!("unexpected extra argument '{a}'"));
        }
    }
    let pane = pane.ok_or_else(|| "migrate requires a <pane> argument".to_string())?;
    let to = to.ok_or_else(|| "migrate requires --to <socket-path>".to_string())?;
    Ok(Cli::Migrate { pane, to })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{parse, usage};

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn socket_flag_space_and_equals_forms() {
        let p = parse(&s(&["--socket", "/tmp/a.sock", "list"])).unwrap();
        assert_eq!(p.socket.as_deref(), Some("/tmp/a.sock"));
        assert_eq!(p.cmd, Cli::List { json: false });
        let p = parse(&s(&["--socket=/tmp/b.sock", "list", "--json"])).unwrap();
        assert_eq!(p.socket.as_deref(), Some("/tmp/b.sock"));
        assert_eq!(p.cmd, Cli::List { json: true });
        // repeats keep the last value; commands still parse after it
        let p = parse(&s(&["--socket", "a", "--socket=b", "instances"])).unwrap();
        assert_eq!(p.socket.as_deref(), Some("b"));
        let p = parse(&s(&["send", "--text", "x", "3"])).unwrap();
        assert_eq!(p.socket, None);
    }

    #[test]
    fn socket_flag_after_subcommand_is_rejected() {
        for argv in [
            vec!["list", "--socket", "/tmp/a.sock"],
            vec!["capture", "--socket=/tmp/a.sock", "a"],
        ] {
            let err = parse(&s(&argv)).unwrap_err();
            assert!(
                err.contains("--socket must appear before the subcommand"),
                "{err}"
            );
        }
    }

    #[test]
    fn socket_flag_requires_a_value() {
        let err = parse(&s(&["--socket"])).unwrap_err();
        assert!(err.contains("requires a value"), "{err}");
        let err = parse(&s(&["--socket="])).unwrap_err();
        assert!(err.contains("requires a value"), "{err}");
        // a bare subcommand-looking token is consumed as the value
        let err = parse(&s(&["--socket", "list"])).unwrap_err();
        assert!(err.starts_with("usage:"), "{err}");
    }

    #[test]
    fn instances_shapes() {
        assert_eq!(
            parse(&s(&["instances"])).unwrap().cmd,
            Cli::Instances {
                json: false,
                all: false
            }
        );
        assert_eq!(
            parse(&s(&["instances", "--all", "--json"])).unwrap().cmd,
            Cli::Instances {
                json: true,
                all: true
            }
        );
        let err = parse(&s(&["instances", "--dead"])).unwrap_err();
        assert!(err.contains("unknown flag '--dead'"), "{err}");
        let err = parse(&s(&["instances", "extra"])).unwrap_err();
        assert!(err.contains("unknown flag 'extra'"), "{err}");
    }

    #[test]
    fn migrate_shapes() {
        let target = "/run/user/1000/terminator-rust/ipc-4242.sock";
        let m = parse(&s(&["migrate", "agent1", "--to", target])).unwrap();
        assert_eq!(m.socket, None);
        assert_eq!(
            m.cmd,
            Cli::Migrate {
                pane: PaneSelector::Name("agent1".into()),
                to: target.to_string(),
            }
        );
        // flag-first and a numeric pane id
        let m = parse(&s(&["migrate", "--to", target, "7"])).unwrap();
        assert_eq!(
            m.cmd,
            Cli::Migrate {
                pane: PaneSelector::Id(7),
                to: target.to_string(),
            }
        );
    }

    #[test]
    fn migrate_errors() {
        let err = parse(&s(&["migrate", "agent1"])).unwrap_err();
        assert!(err.contains("migrate requires --to"), "{err}");
        let err = parse(&s(&["migrate"])).unwrap_err();
        assert!(err.contains("<pane>"), "{err}");
        let err = parse(&s(&["migrate", "agent1", "--to"])).unwrap_err();
        assert!(err.contains("requires a value"), "{err}");
        let err = parse(&s(&["migrate", "--to", "x"])).unwrap_err();
        assert!(err.contains("<pane>"), "{err}");
        let err = parse(&s(&["migrate", "a", "b", "--to", "x"])).unwrap_err();
        assert!(err.contains("extra argument"), "{err}");
        let err = parse(&s(&["migrate", "a", "--from", "x", "--to", "y"])).unwrap_err();
        assert!(err.contains("unknown flag '--from'"), "{err}");
    }

    #[test]
    fn usage_documents_the_new_surface() {
        let u = usage();
        assert!(u.contains("--socket <path>"), "{u}");
        assert!(u.contains("instances"), "{u}");
        assert!(u.contains("migrate"), "{u}");
        assert!(u.contains("--to <socket-path>"), "{u}");
    }
}
