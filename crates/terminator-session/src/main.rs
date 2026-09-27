//! A small remote PTY keeper. One daemon owns one shell and one named socket.

mod client;
mod path;
mod protocol;
mod pty;
mod server;

use anyhow::{bail, Result};

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("attach") => {
            let name = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing session name"))?;
            path::validate_name(&name)?;
            let mut title = name.clone();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--title" => {
                        title = args
                            .next()
                            .ok_or_else(|| anyhow::anyhow!("missing title"))?
                    }
                    _ => bail!("unknown argument: {arg}"),
                }
            }
            client::attach(&name, &title)
        }
        Some("list") if args.next().is_none() => client::list(),
        Some("__serve") => {
            let name = args.next().ok_or_else(|| anyhow::anyhow!("missing name"))?;
            path::validate_name(&name)?;
            let title = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing title"))?;
            let cwd = args.next().ok_or_else(|| anyhow::anyhow!("missing cwd"))?;
            server::serve(&name, &title, &cwd)
        }
        _ => bail!("usage: terminator-session attach NAME [--title TITLE] | list"),
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("terminator-session: {e:#}");
        std::process::exit(1);
    }
}
