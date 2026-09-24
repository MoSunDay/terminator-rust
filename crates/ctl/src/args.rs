//! Pure argv parsing for terminator-ctl: no I/O, no exit codes. Bad input
//! becomes `Err(String)` and `main` decides how to present it. Help is an
//! `Err` carrying the usage text (starts with "usage:"), which keeps the
//! enum free of a `Help` variant nobody dispatches on.

use ipc_proto::PaneSelector;

const USAGE_TEXT: &str = "\
usage: terminator-ctl [--socket <path>] <command> [options]

commands:
  list [--json]                              list panes
  capture <pane> [--lines N] [--json]        show a pane's visible screen
  send <pane> --text <text> [--bracketed]    type text into a pane
  instances [--json] [--all]                 list running instances (control
                                             sockets in the runtime dir;
                                             --all keeps dead ones)
  migrate <pane> --to <socket-path>          move <pane>'s whole tab to the
                                             instance listening on <socket>
  oc link <pane> [--session ID]            pin pane -> opencoder store
  oc submit <pane> <text> [--delivery steer|queue] [--wait SECS]
                                           append a pending input for the TUI
  oc status <pane>                         pending inputs + receipts
  oc sessions <pane>                       sessions in the pane's store

options:
  --socket <path>    control socket to talk to (must come before the
                     command; default: $TERMINATOR_SOCK, then the usual
                     runtime/config locations)

<pane> is a pane name (manual title) or a numeric pane id
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    List {
        json: bool,
    },
    Capture {
        pane: PaneSelector,
        lines: u32,
        json: bool,
    },
    Send {
        pane: PaneSelector,
        text: String,
        bracketed: bool,
    },
    /// `instances`: roster of terminator-rust control sockets in the
    /// runtime dir, probed for liveness (dead ones only with `all`).
    Instances {
        json: bool,
        all: bool,
    },
    /// `migrate`: hand the tab containing `pane` to the instance on `to`.
    Migrate {
        pane: PaneSelector,
        to: String,
    },
    Oc(Vec<String>),
}

/// One parsed command line: the global `--socket` override plus the
/// subcommand. The socket is separate from [`Cli`] so the enum stays a
/// pure description of the command itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// `--socket <path>`, only valid before the subcommand.
    pub socket: Option<String>,
    pub cmd: Cli,
}

/// `args` excludes argv[0]; the first non-`--socket` element is the
/// subcommand. Flags may appear in any order around the pane positional.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let (socket, args) = crate::args_extra::strip_global_socket(args)?;
    let first = args.first().ok_or_else(|| usage().to_string())?;
    if first == "-h" || first == "--help" {
        return Err(usage().to_string());
    }
    let rest = &args[1..];
    let cmd = match first.as_str() {
        "list" => parse_list(rest)?,
        "capture" => parse_capture(rest)?,
        "send" => parse_send(rest)?,
        "instances" => crate::args_extra::parse_instances(rest)?,
        "migrate" => crate::args_extra::parse_migrate(rest)?,
        "oc" => Cli::Oc(rest.to_vec()),
        other => return Err(format!("unknown subcommand '{other}'")),
    };
    Ok(Parsed { socket, cmd })
}

pub fn usage() -> &'static str {
    USAGE_TEXT
}

fn parse_list(rest: &[String]) -> Result<Cli, String> {
    let mut json = false;
    for a in rest {
        match a.as_str() {
            "--json" => json = true,
            other => return Err(unknown_flag(other)),
        }
    }
    Ok(Cli::List { json })
}

fn parse_capture(rest: &[String]) -> Result<Cli, String> {
    let mut pane: Option<PaneSelector> = None;
    let mut lines: u32 = 80;
    let mut json = false;
    for i in flag_walk(rest) {
        let a = rest[i].as_str();
        if a == "--json" {
            json = true;
        } else if a == "--lines" {
            let v = value_of(rest, i, "--lines")?;
            lines = v
                .parse()
                .map_err(|_| format!("bad --lines '{v}' (expected a number)"))?;
        } else if a.starts_with("--") {
            return Err(unknown_flag(a));
        } else if pane.replace(parse_pane(a)).is_some() {
            return Err(format!("unexpected extra argument '{a}'"));
        }
    }
    let pane = pane.ok_or_else(|| "capture requires a <pane> argument".to_string())?;
    Ok(Cli::Capture { pane, lines, json })
}

fn parse_send(rest: &[String]) -> Result<Cli, String> {
    let mut pane: Option<PaneSelector> = None;
    let mut text: Option<String> = None;
    let mut bracketed = false;
    for i in flag_walk(rest) {
        let a = rest[i].as_str();
        if a == "--bracketed" {
            bracketed = true;
        } else if a == "--text" {
            text = Some(value_of(rest, i, "--text")?);
        } else if a.starts_with("--") {
            return Err(unknown_flag(a));
        } else if pane.replace(parse_pane(a)).is_some() {
            return Err(format!("unexpected extra argument '{a}'"));
        }
    }
    let pane = pane.ok_or_else(|| "send requires a <pane> argument".to_string())?;
    let text = text.ok_or_else(|| "send requires --text <text>".to_string())?;
    Ok(Cli::Send {
        pane,
        text,
        bracketed,
    })
}

/// Indices of the tokens to interpret: flags that consume a value are
/// yielded once, and their value index is skipped (read via `value_of`).
/// Shared with `args_extra` (the multi-instance command parsers).
pub(crate) fn flag_walk(rest: &[String]) -> Vec<usize> {
    let mut skip_next = false;
    (0..rest.len())
        .filter(|i| {
            if skip_next {
                skip_next = false;
                return false;
            }
            if is_valued_flag(&rest[*i]) {
                skip_next = true;
            }
            true
        })
        .collect()
}

fn is_valued_flag(a: &str) -> bool {
    a == "--lines" || a == "--text" || a == "--to"
}

/// The value token following valued-flag index `i`.
pub(crate) fn value_of(rest: &[String], i: usize, flag: &str) -> Result<String, String> {
    rest.get(i + 1)
        .cloned()
        .ok_or_else(|| format!("flag {flag} requires a value"))
}

/// Pure decimal digits -> pane id, anything else -> pane name.
pub(crate) fn parse_pane(s: &str) -> PaneSelector {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(id) = s.parse() {
            return PaneSelector::Id(id);
        }
    }
    PaneSelector::Name(s.to_string())
}

pub(crate) fn unknown_flag(f: &str) -> String {
    // The global flag is only valid before the subcommand; say so instead
    // of the generic unknown-flag text when it shows up too late.
    if f == "--socket" || f.starts_with("--socket=") {
        return "--socket must appear before the subcommand".to_string();
    }
    format!("unknown flag '{f}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn list_flags() {
        assert_eq!(parse(&s(&["list"])).unwrap().cmd, Cli::List { json: false });
        assert_eq!(
            parse(&s(&["list", "--json"])).unwrap().cmd,
            Cli::List { json: true }
        );
        assert!(parse(&s(&["list", "--pretty"]))
            .unwrap_err()
            .contains("unknown flag"));
    }

    #[test]
    fn capture_flag_order_and_defaults() {
        let plain = parse(&s(&["capture", "agent1"])).unwrap().cmd;
        assert_eq!(
            plain,
            Cli::Capture {
                pane: PaneSelector::Name("agent1".into()),
                lines: 80,
                json: false
            }
        );
        let before = parse(&s(&["capture", "--json", "--lines", "10", "7"]))
            .unwrap()
            .cmd;
        assert_eq!(
            before,
            Cli::Capture {
                pane: PaneSelector::Id(7),
                lines: 10,
                json: true
            }
        );
        let mixed = parse(&s(&["capture", "7", "--lines", "3", "--json"]))
            .unwrap()
            .cmd;
        assert_eq!(
            mixed,
            Cli::Capture {
                pane: PaneSelector::Id(7),
                lines: 3,
                json: true
            }
        );
    }

    #[test]
    fn pane_id_vs_name() {
        assert_eq!(parse_pane("12"), PaneSelector::Id(12));
        assert_eq!(parse_pane("0012"), PaneSelector::Id(12));
        assert_eq!(parse_pane("0"), PaneSelector::Id(0));
        assert_eq!(parse_pane("a12"), PaneSelector::Name("a12".into()));
        assert_eq!(parse_pane("+5"), PaneSelector::Name("+5".into()));
        assert_eq!(parse_pane("12x"), PaneSelector::Name("12x".into()));
    }

    #[test]
    fn send_text_and_bracketed() {
        let ok = parse(&s(&["send", "agent1", "--text", "hello world"]))
            .unwrap()
            .cmd;
        assert_eq!(
            ok,
            Cli::Send {
                pane: PaneSelector::Name("agent1".into()),
                text: "hello world".into(),
                bracketed: false,
            }
        );
        let br = parse(&s(&["send", "--bracketed", "--text", "x", "3"]))
            .unwrap()
            .cmd;
        assert_eq!(
            br,
            Cli::Send {
                pane: PaneSelector::Id(3),
                text: "x".into(),
                bracketed: true
            }
        );
        let err = parse(&s(&["send", "agent1"])).unwrap_err();
        assert!(err.contains("--text"), "{err}");
        let err = parse(&s(&["send", "agent1", "--text"])).unwrap_err();
        assert!(err.contains("requires a value"), "{err}");
    }

    #[test]
    fn bad_lines() {
        let err = parse(&s(&["capture", "a", "--lines", "abc"])).unwrap_err();
        assert!(err.contains("bad --lines"), "{err}");
        let err = parse(&s(&["capture", "a", "--lines"])).unwrap_err();
        assert!(err.contains("requires a value"), "{err}");
        let err = parse(&s(&["capture", "a", "--lines", "-1"])).unwrap_err();
        assert!(err.contains("bad --lines"), "{err}");
    }

    #[test]
    fn unknown_things() {
        let err = parse(&s(&["explode"])).unwrap_err();
        assert!(err.contains("unknown subcommand 'explode'"), "{err}");
        let err = parse(&s(&["capture", "--verbose", "a"])).unwrap_err();
        assert!(err.contains("unknown flag '--verbose'"), "{err}");
        let err = parse(&s(&["capture"])).unwrap_err();
        assert!(err.contains("<pane>"), "{err}");
        let err = parse(&s(&["capture", "a", "b"])).unwrap_err();
        assert!(err.contains("extra argument"), "{err}");
    }

    #[test]
    fn help_and_oc() {
        for h in ["-h", "--help"] {
            let err = parse(&s(&[h])).unwrap_err();
            assert!(err.starts_with("usage:"), "{err}");
        }
        let empty = parse(&[]).unwrap_err();
        assert!(empty.starts_with("usage:"), "{empty}");
        assert_eq!(
            parse(&s(&["oc", "sessions", "--all"])).unwrap().cmd,
            Cli::Oc(s(&["sessions", "--all"]))
        );
    }
}
