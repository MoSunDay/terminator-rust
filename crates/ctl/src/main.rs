//! `terminator-ctl`: command-line client for terminator-rust's UDS control
//! socket (wire shape in `ipc-proto`). This file is dispatch only: `args`
//! parses argv, `uds` speaks the protocol, `format` shapes output, and
//! `cmd_oc`/`procfs` are the landing zone for opencoder tooling.

mod args;
mod args_extra;
mod cmd_oc;
mod format;
mod procfs;
#[cfg(target_os = "linux")]
mod procfs_linux;
#[cfg(target_os = "macos")]
mod procfs_macos;
mod sidecar;
mod uds;

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::time::Duration;

use anyhow::{anyhow, Result};
use ipc_proto::{Request, Response, DEFAULT_TIMEOUT_SECS};

use crate::args::Cli;

fn main() {
    // args_os + lossy: non-UTF-8 argv (e.g. a latin-1 filename from shell
    // completion) must not panic; the lossy name just fails the pane
    // lookup with the normal not-found error.
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let cli = match args::parse(&argv) {
        Ok(parsed) => parsed,
        Err(e) => {
            // Help / empty argv already *is* the usage text; anything else
            // gets the reason first, usage after.
            if !e.starts_with("usage:") {
                eprintln!("error: {e}\n");
            }
            eprint!("{}", args::usage());
            std::process::exit(2);
        }
    };
    if let Err(e) = dispatch(cli.cmd, cli.socket.as_deref()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli, socket: Option<&str>) -> Result<()> {
    let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
    match cli {
        Cli::List { json } => {
            let panes = match uds::request_flagged(socket, &Request::List, timeout)? {
                Response::List { panes } => panes,
                other => return Err(unexpected(&other)),
            };
            let out = if json {
                format::pane_json(&panes)
            } else {
                format::pane_table(&panes)
            };
            print!("{out}");
        }
        Cli::Capture { pane, lines, json } => {
            let cap =
                match uds::request_flagged(socket, &Request::Capture { pane, lines }, timeout)? {
                    Response::Capture(out) => out,
                    other => return Err(unexpected(&other)),
                };
            let out = if json {
                format::capture_json(&cap)
            } else {
                format::capture_text(&cap)
            };
            print!("{out}");
        }
        Cli::Send {
            pane,
            text,
            bracketed,
        } => {
            let bytes = match uds::request_flagged(
                socket,
                &Request::Write {
                    pane,
                    text,
                    bracketed,
                },
                timeout,
            )? {
                Response::Written { bytes } => bytes,
                other => return Err(unexpected(&other)),
            };
            println!("wrote {bytes} bytes");
        }
        Cli::Notice => notice_cmd(),
        Cli::Instances { json, all } => {
            let mut rows = Vec::new();
            for path in uds::discover() {
                let answer = uds::probe(&path, uds::PROBE_TIMEOUT);
                if !all && answer.is_none() {
                    continue;
                }
                rows.push(format::instance_row(&path, answer.as_ref()));
            }
            let out = if json {
                format::instances_json(&rows)
            } else {
                format::instances_table(&rows)
            };
            print!("{out}");
        }
        Cli::Migrate { pane, to } => {
            let answer = uds::request_flagged(
                socket,
                &Request::MigrateOut {
                    pane,
                    target: to.clone(),
                },
                timeout,
            )?;
            match answer {
                Response::Migrated { panes } => println!("migrated {panes} pane(s) to {to}"),
                other => return Err(unexpected(&other)),
            }
        }
        Cli::Oc(rest) => cmd_oc::dispatch_oc(&rest, socket)?,
    }
    Ok(())
}

/// Where the OSC 9 bytes go: the pane's controlling terminal when one can
/// be opened, stdout otherwise. Kept as data so the fallback choice stays
/// a pure, testable decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeSink {
    /// `/dev/tty` — the pane's own controlling terminal. The bytes arrive
    /// as terminal OUTPUT, so the VT engine fires the badge directly; via
    /// a terminator-session keeper they also enter the replay history and
    /// re-mark the pane on reattach.
    Tty,
    /// stdout — fallback for callers with no controlling terminal at all
    /// (hooks usually redirect stdout, so this is never the primary path).
    Stdout,
}

/// `/dev/tty` is opened for writing only: the bytes are terminal output,
/// not user input, and O_WRONLY can never steal the tty.
const TTY_PATH: &str = "/dev/tty";

/// Pure choice of [`NoticeSink`]: the tty whenever it opens, stdout
/// otherwise. Injectable (`tty_openable`) so tests pin the fallback
/// without touching a real tty.
fn emit_target(tty_openable: bool) -> NoticeSink {
    if tty_openable {
        NoticeSink::Tty
    } else {
        NoticeSink::Stdout
    }
}

/// Injectable writer: emits EXACTLY the canonical OSC 9 desktop-
/// notification bytes ([`ipc_proto::notice_osc`]) to whatever sink the
/// caller chooses — a real tty, stdout, or a test buffer.
fn write_notice<W: Write>(w: &mut W) -> io::Result<()> {
    w.write_all(&ipc_proto::notice_osc())?;
    w.flush()
}

/// `notice`: one transport — write the canonical OSC 9 bytes to
/// `/dev/tty` (the pane's controlling terminal), stdout only when no tty
/// can be opened. Best-effort by contract: a notification must never
/// break a hook, so write failures surface as a short stderr note and
/// the command still exits 0 (errors here cannot reach `main`'s exit-1
/// path).
fn notice_cmd() {
    let mut tty = OpenOptions::new().write(true).open(TTY_PATH).ok();
    let sink = emit_target(tty.is_some());
    let outcome = match (&mut tty, sink) {
        (Some(w), NoticeSink::Tty) => write_notice(w),
        _ => write_notice(&mut io::stdout().lock()),
    };
    if let Err(e) = outcome {
        eprintln!("notice: cannot write the OSC 9 bytes to {sink:?} ({e})");
    }
}

/// `uds::request` already folds `Response::Error` into `Err`; reaching this
/// means the app answered a different command than we asked — protocol bug.
fn unexpected(r: &Response) -> anyhow::Error {
    anyhow!("unexpected response kind: {r:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_writes_exactly_the_canonical_osc9_bytes() {
        // The injectable writer against an in-memory sink: byte-for-byte
        // the shared canonical payload, nothing else (no newline, no
        // keeper argv, no decoration).
        let mut buf = Vec::new();
        write_notice(&mut buf).expect("writing to a Vec cannot fail");
        assert_eq!(buf, ipc_proto::notice_osc());
        assert_eq!(buf, b"\x1b]9;terminator-rust notice\x07".to_vec());
    }

    #[test]
    fn notice_falls_back_to_stdout_only_without_a_tty() {
        // Pure choice, no real tty involved: the pane's controlling
        // terminal wins whenever it opens, stdout only otherwise.
        assert_eq!(emit_target(true), NoticeSink::Tty);
        assert_eq!(emit_target(false), NoticeSink::Stdout);
    }
}
