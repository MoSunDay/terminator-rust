//! `terminator-ctl oc ...`: submit prompts straight into an OpenCoder
//! per-workdir store so the OpenCoder TUI (the sole runner) claims them at
//! its turn (steer) / idle (queue) boundaries. Discovery: pane pid ->
//! /proc descendants -> opencoder process -> fd table -> opencoder.db.
//! Pane-name -> (db, session) pins live in `sidecar`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use ipc_proto::{PaneInfo, PaneSelector, Request, Response};

use crate::sidecar::{self, Link};
use crate::{procfs, uds};

const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Oc {
    Link {
        pane: String,
        session: Option<String>,
    },
    Submit {
        pane: String,
        text: String,
        delivery: String,
        wait: u64,
    },
    Status {
        pane: String,
    },
    Sessions {
        pane: String,
    },
    Unlink {
        pane: String,
    },
}

pub fn usage() -> &'static str {
    "oc link <pane> [--session ID]        pin pane -> opencoder db (+session)
oc unlink <pane>                       forget a pin
oc submit <pane> <text> [--delivery steer|queue] [--wait SECS]
oc status <pane>                       pending inputs + consumption receipts
oc sessions <pane>                     sessions in the pane's store"
}

/// Pure argv parsing for `oc` (everything after the `oc` token).
pub fn parse_oc(rest: &[String]) -> Result<Oc, String> {
    let sub = rest.first().ok_or_else(|| usage().to_string())?.as_str();
    let rest = &rest[1..];
    let mut positional: Vec<&String> = Vec::new();
    let mut session: Option<String> = None;
    let mut delivery = "steer".to_string();
    let mut wait: u64 = 0;
    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].as_str();
        match a {
            "--session" => session = Some(flag_value(rest, &mut i, a)?.to_string()),
            "--delivery" => {
                delivery = flag_value(rest, &mut i, a)?.to_string();
                if delivery != "steer" && delivery != "queue" {
                    return Err(format!(
                        "--delivery must be steer or queue, got '{delivery}'"
                    ));
                }
            }
            "--wait" => {
                wait = flag_value(rest, &mut i, a)?
                    .parse()
                    .map_err(|_| "--wait must be seconds".to_string())?;
            }
            _ if a.starts_with("--") => return Err(format!("unknown flag '{a}'")),
            _ => positional.push(&rest[i]),
        }
        i += 1;
    }
    let pane = positional.first().map(|s| s.to_string());
    let need = |name: &str| -> Result<String, String> {
        pane.clone()
            .ok_or_else(|| format!("missing <pane> for 'oc {name}'"))
    };
    match sub {
        "link" => Ok(Oc::Link {
            pane: need("link")?,
            session,
        }),
        "unlink" => Ok(Oc::Unlink {
            pane: need("unlink")?,
        }),
        "submit" => {
            let text = positional
                .get(1)
                .map(|s| s.to_string())
                .ok_or_else(|| "missing <text> for 'oc submit'".to_string())?;
            Ok(Oc::Submit {
                pane: need("submit")?,
                text,
                delivery,
                wait,
            })
        }
        "status" => Ok(Oc::Status {
            pane: need("status")?,
        }),
        "sessions" => Ok(Oc::Sessions {
            pane: need("sessions")?,
        }),
        "-h" | "--help" => Err(usage().to_string()),
        other => Err(format!("unknown oc subcommand '{other}'")),
    }
}

/// Value of `rest[*i + 1]`, advancing `*i` past it (the loop's own `i += 1`
/// then lands on the next flag/positional).
fn flag_value<'a>(rest: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    let v = rest
        .get(*i + 1)
        .map(|v| v.as_str())
        .ok_or_else(|| format!("{flag} needs a value"))?;
    *i += 1;
    Ok(v)
}

pub fn dispatch_oc(rest: &[String]) -> Result<()> {
    match parse_oc(rest).map_err(|e| anyhow!("{e}"))? {
        Oc::Link { pane, session } => cmd_link(&pane, session.as_deref()),
        Oc::Unlink { pane } => cmd_unlink(&pane),
        Oc::Submit {
            pane,
            text,
            delivery,
            wait,
        } => cmd_submit(&pane, &text, &delivery, wait),
        Oc::Status { pane } => cmd_status(&pane),
        Oc::Sessions { pane } => cmd_sessions(&pane),
    }
}

/// Pane address from the CLI -> live PaneInfo via the app.
fn pane_info(selector: &PaneSelector) -> Result<PaneInfo> {
    let panes = match uds::request(&Request::List, TIMEOUT)? {
        Response::List { panes } => panes,
        other => return Err(anyhow!("unexpected response: {other:?}")),
    };
    panes
        .into_iter()
        .find(|p| match selector {
            PaneSelector::Name(n) => p.name.as_deref() == Some(n.as_str()),
            PaneSelector::Id(id) => p.id == *id,
        })
        .ok_or_else(|| anyhow!("no pane matching {selector:?} (named panes: double-click a title)"))
}

/// Resolve (db, session) for a pane: sidecar pin first, then /proc discovery.
fn resolve(pane: &str, info: &PaneInfo) -> Result<(PathBuf, String)> {
    let side = sidecar::load(&sidecar::sidecar_path())?;
    if let Some(link) = side.links.get(pane) {
        let db = PathBuf::from(&link.db);
        let store = oc_store::db::open_ro(&db)?;
        let session = match &link.session {
            Some(s) => {
                if !store.session_exists(s)? {
                    bail!("pinned session {s} no longer exists in {}", db.display());
                }
                s.clone()
            }
            None => store
                .latest_session()?
                .map(|s| s.id)
                .ok_or_else(|| anyhow!("store {} has no sessions", db.display()))?,
        };
        return Ok((db, session));
    }
    let found = procfs::find_opencoder(info.pid)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow!("no opencoder process found under pane '{pane}' (pid {}); run `oc link` while its TUI is alive or after spawning it", info.pid))?;
    let store = oc_store::db::open_ro(&found.db)?;
    let session = store
        .latest_session()?
        .map(|s| s.id)
        .ok_or_else(|| anyhow!("store {} has no sessions", found.db.display()))?;
    Ok((found.db, session))
}

fn cmd_link(pane: &str, session: Option<&str>) -> Result<()> {
    let info = pane_info(&PaneSelector::Name(pane.to_string()))?;
    let found = procfs::find_opencoder(info.pid)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow!("no opencoder process found under pane '{pane}'"))?;
    let pinned = match session {
        Some(id) => {
            let store = oc_store::db::open_ro(&found.db)?;
            if !store.session_exists(id)? {
                bail!("session {id} not found in {}", found.db.display());
            }
            Some(id.to_string())
        }
        None => None,
    };
    let link = Link {
        db: found.db.display().to_string(),
        session: pinned,
    };
    println!("linked pane '{pane}' -> {}", found.db.display());
    match &link.session {
        Some(id) => println!("session pinned: {id}"),
        None => println!("session: latest at submit time"),
    }
    sidecar::set(&sidecar::sidecar_path(), pane, link)?;
    Ok(())
}

fn cmd_unlink(pane: &str) -> Result<()> {
    sidecar::remove(&sidecar::sidecar_path(), pane)?;
    println!("unlinked pane '{pane}'");
    Ok(())
}

fn cmd_submit(pane: &str, text: &str, delivery: &str, wait: u64) -> Result<()> {
    let info = pane_info(&PaneSelector::Name(pane.to_string()))?;
    let (db, session) = resolve(pane, &info)?;
    let store = oc_store::db::open_rw(&db)?;
    let seq = store.insert_input(&session, delivery, text)?;
    println!("submitted #{seq} ({delivery}) to session {session}");
    println!("  db: {}", db.display());
    if wait == 0 {
        return Ok(());
    }
    wait_consumed(&db, &session, seq, wait)
}

/// Poll receipts until our seq shows up or the deadline; a timeout is
/// reported honestly (exit 1) instead of pretending success.
fn wait_consumed(db: &std::path::Path, session: &str, seq: i64, wait: u64) -> Result<()> {
    let started = Instant::now();
    let deadline = Duration::from_secs(wait);
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let store = oc_store::db::open_ro(db)?;
        if let Some(hit) = store.receipts(session, 50)?.iter().find(|r| r.seq == seq) {
            println!(
                "consumed after {:.1}s ({})",
                started.elapsed().as_secs_f32(),
                hit.kind
            );
            return Ok(());
        }
        if started.elapsed() >= deadline {
            bail!("still pending after {wait}s (steers land at turn boundaries; the TUI must be running)");
        }
    }
}

fn cmd_status(pane: &str) -> Result<()> {
    let info = pane_info(&PaneSelector::Name(pane.to_string()))?;
    let (db, session) = resolve(pane, &info)?;
    let store = oc_store::db::open_ro(&db)?;
    println!("session {session}");
    println!("  db: {}", db.display());
    let pending = store.pending(&session)?;
    if pending.is_empty() {
        println!("pending: none");
    } else {
        println!("pending:");
        for p in &pending {
            println!(
                "  #{} {} adm={} \"{}\"",
                p.seq,
                p.delivery,
                p.admitted_seq,
                truncate(&p.prompt, 72)
            );
        }
    }
    let receipts = store.receipts(&session, 20)?;
    if receipts.is_empty() {
        println!("receipts: none yet");
    } else {
        println!("receipts (latest {}):", receipts.len());
        for r in receipts.iter().rev() {
            println!("  #{} {} \"{}\"", r.seq, r.kind, truncate(&r.text, 72));
        }
    }
    Ok(())
}

fn cmd_sessions(pane: &str) -> Result<()> {
    let info = pane_info(&PaneSelector::Name(pane.to_string()))?;
    let (db, _) = resolve(pane, &info)?;
    let store = oc_store::db::open_ro(&db)?;
    let rows = store.sessions(20)?;
    if rows.is_empty() {
        println!("no sessions in {}", db.display());
        return Ok(());
    }
    for r in rows {
        println!(
            "{}  updated={}  {}",
            r.id,
            r.updated_at,
            r.title.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn parses_all_subcommands() {
        let p = |v: &[&str]| parse_oc(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(
            p(&["link", "a1"]).unwrap(),
            Oc::Link {
                pane: s("a1"),
                session: None
            }
        );
        assert_eq!(
            p(&["link", "a1", "--session", "01M"]).unwrap(),
            Oc::Link {
                pane: s("a1"),
                session: Some(s("01M"))
            }
        );
        assert_eq!(
            p(&[
                "submit",
                "a1",
                "fix it",
                "--delivery",
                "queue",
                "--wait",
                "30"
            ])
            .unwrap(),
            Oc::Submit {
                pane: s("a1"),
                text: s("fix it"),
                delivery: s("queue"),
                wait: 30
            }
        );
        assert_eq!(
            p(&["submit", "--delivery", "steer", "a1", "hi"]).unwrap(),
            Oc::Submit {
                pane: s("a1"),
                text: s("hi"),
                delivery: s("steer"),
                wait: 0
            }
        );
        assert_eq!(p(&["status", "a1"]).unwrap(), Oc::Status { pane: s("a1") });
        assert_eq!(
            p(&["sessions", "a1"]).unwrap(),
            Oc::Sessions { pane: s("a1") }
        );
        assert_eq!(p(&["unlink", "a1"]).unwrap(), Oc::Unlink { pane: s("a1") });
        assert!(p(&["submit", "a1"]).is_err()); // no text
        assert!(p(&["submit", "a1", "t", "--delivery", "urgent"]).is_err());
        assert!(p(&["submit", "a1", "t", "--wait", "x"]).is_err());
        assert!(p(&[]).is_err());
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdefg", 6), "abcde...");
    }
}
