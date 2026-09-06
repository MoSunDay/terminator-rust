//! `terminator-ctl`: command-line client for terminator-rust's UDS control
//! socket (wire shape in `ipc-proto`). This file is dispatch only: `args`
//! parses argv, `uds` speaks the protocol, `format` shapes output, and
//! `cmd_oc`/`procfs` are the landing zone for opencoder tooling.

mod args;
mod cmd_oc;
mod format;
mod procfs;
mod sidecar;
mod uds;

use std::time::Duration;

use anyhow::{anyhow, Result};
use ipc_proto::{Request, Response, DEFAULT_TIMEOUT_SECS};

use crate::args::Cli;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cli = match args::parse(&argv) {
        Ok(cli) => cli,
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
    if let Err(e) = dispatch(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli) -> Result<()> {
    let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
    match cli {
        Cli::List { json } => {
            let panes = match uds::request(&Request::List, timeout)? {
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
            let cap = match uds::request(&Request::Capture { pane, lines }, timeout)? {
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
            let bytes = match uds::request(
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
        Cli::Oc(rest) => cmd_oc::dispatch_oc(&rest)?,
    }
    Ok(())
}

/// `uds::request` already folds `Response::Error` into `Err`; reaching this
/// means the app answered a different command than we asked — protocol bug.
fn unexpected(r: &Response) -> anyhow::Error {
    anyhow!("unexpected response kind: {r:?}")
}
