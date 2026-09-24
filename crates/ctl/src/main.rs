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

/// `uds::request` already folds `Response::Error` into `Err`; reaching this
/// means the app answered a different command than we asked — protocol bug.
fn unexpected(r: &Response) -> anyhow::Error {
    anyhow!("unexpected response kind: {r:?}")
}
