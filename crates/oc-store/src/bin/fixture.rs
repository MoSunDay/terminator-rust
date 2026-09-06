//! Dev/e2e helper: fabricate an empty OpenCoder-compatible per-workdir store
//! (exact chat-table DDL, user_version 18) with one session row, so `oc`
//! tooling can be exercised without a live opencoder install.
//!
//! Usage:
//!   oc-store-fixture <db-path> [session-id]              create/reset store
//!   oc-store-fixture <db-path> consume <session> <seq> <steer|queue>
//!                                                         emulate the TUI claim

use std::path::Path;

use rusqlite::Connection;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(db) = args.next() else {
        eprintln!("usage: oc-store-fixture <db-path> [session-id]");
        eprintln!("       oc-store-fixture <db-path> consume <session> <seq> <steer|queue>");
        std::process::exit(2);
    };
    let res = match args.next().as_deref() {
        Some("consume") => {
            let rest: Vec<String> = args.collect();
            if rest.len() != 3 {
                eprintln!("consume needs <session> <seq> <steer|queue>");
                std::process::exit(2);
            }
            consume(&db, &rest[0], rest[1].parse().unwrap_or(-1), &rest[2])
        }
        session => create(&db, session.unwrap_or("01HXFIXTURESESSION00")),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Emulate the OpenCoder TUI's claim: mark the row promoted+recorded and
/// append the corresponding *_consumed session event.
fn consume(db: &str, session: &str, seq: i64, delivery: &str) -> anyhow::Result<()> {
    if delivery != "steer" && delivery != "queue" {
        anyhow::bail!("delivery must be steer|queue");
    }
    let conn = Connection::open(db)?;
    let prompt: String = conn.query_row(
        "SELECT prompt FROM session_inputs WHERE seq = ?1",
        [seq],
        |r| r.get(0),
    )?;
    conn.execute(
        "UPDATE session_inputs SET promoted_seq = 1, recorded = 1 WHERE seq = ?1",
        [seq],
    )?;
    conn.execute(
        "INSERT INTO session_events (session_id, type, payload_json, ts)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            session,
            format!("{delivery}_consumed"),
            serde_json::json!({"seq": seq, "text": prompt}).to_string(),
            1_780_000_000_000i64
        ],
    )?;
    println!("consumed #{seq} ({delivery})");
    Ok(())
}

/// Create (or reset) the fixture store. DDL is verbatim from the OpenCoder
/// schema bootstrap so schema guards in `db` accept it.
fn create(path: &str, session: &str) -> anyhow::Result<()> {
    if let Some(dir) = Path::new(path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "DROP TABLE IF EXISTS session_events;
         DROP TABLE IF EXISTS session_inputs;
         DROP TABLE IF EXISTS sessions;
         CREATE TABLE sessions (
           id           TEXT PRIMARY KEY,
           title        TEXT,
           agent        TEXT,
           model        TEXT,
           workdir_hash TEXT,
           created_at   INTEGER NOT NULL,
           updated_at   INTEGER NOT NULL,
           summary      TEXT,
           summary_seq      INTEGER,
           summary_images_json TEXT,
           handoff_seq  INTEGER,
           handoff_plan TEXT,
           skill        TEXT,
           task_type    TEXT NOT NULL DEFAULT 'parent',
           requirement  TEXT,
           plan_snapshot TEXT,
           plan_input_count INTEGER NOT NULL DEFAULT 0,
           autopilot_mode TEXT);
         CREATE TABLE session_inputs (
           seq          INTEGER PRIMARY KEY AUTOINCREMENT,
           id           TEXT NOT NULL,
           session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           delivery     TEXT NOT NULL,
           prompt       TEXT NOT NULL,
           images_json  TEXT NOT NULL DEFAULT '[]',
           display_text TEXT,
           admitted_seq INTEGER NOT NULL,
           promoted_seq INTEGER,
           recorded     INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE session_events (
           seq         INTEGER PRIMARY KEY AUTOINCREMENT,
           session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           type        TEXT NOT NULL,
           payload_json TEXT NOT NULL,
           sse_kind    TEXT,
           ts          INTEGER NOT NULL);
         PRAGMA user_version=18;
         PRAGMA journal_mode=WAL;",
    )?;
    let now = 1_780_000_000_000i64;
    conn.execute(
        "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, 'fixture', ?2, ?2)",
        rusqlite::params![session, now],
    )?;
    Ok(())
}
