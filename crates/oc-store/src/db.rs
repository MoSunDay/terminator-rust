//! Read/append access to one OpenCoder per-workdir SQLite store.
//!
//! The OpenCoder TUI is the sole runner: terminator-rust only appends
//! pending inputs ([`Store::insert_input`]) that the TUI claims at turn
//! boundaries (`steer`) or idle boundaries (`queue`), and reads back the
//! `steer_consumed`/`queue_consumed` receipt events.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

/// The schema generation this crate understands (mirrors the real store).
pub const SCHEMA_VERSION: i64 = 18;

const REQUIRED_INPUT_COLUMNS: [&str; 10] = [
    "seq",
    "id",
    "session_id",
    "delivery",
    "prompt",
    "images_json",
    "display_text",
    "admitted_seq",
    "promoted_seq",
    "recorded",
];

const REQUIRED_SESSION_COLUMNS: [&str; 3] = ["id", "updated_at", "title"];

/// One open store. `rusqlite::Connection` is `!Sync`, so a `Store` is owned
/// single-threaded per CLI invocation and never shared across threads.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
    path: PathBuf,
}

/// Open an existing store read-write. Never creates the file; refuses
/// anything whose schema is not exactly the expected generation.
pub fn open_rw(path: &Path) -> Result<Store> {
    if !path.is_file() {
        bail!("no opencoder store at {}", path.display());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)
        .with_context(|| format!("open {} read-write", path.display()))?;
    conn.execute_batch("PRAGMA busy_timeout=30000;")
        .context("set busy_timeout=30000")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .context("set foreign_keys=ON")?;
    // journal_mode is best-effort (fails on some filesystems): log-ignore.
    let _ = conn.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()));
    check_schema(&conn, path)?;
    Ok(Store {
        conn,
        path: path.to_path_buf(),
    })
}

/// Open an existing store read-only. Same guards, minus WAL (cannot be set
/// on a read-only connection).
pub fn open_ro(path: &Path) -> Result<Store> {
    if !path.is_file() {
        bail!("no opencoder store at {}", path.display());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)
        .with_context(|| format!("open {} read-only", path.display()))?;
    conn.execute_batch("PRAGMA busy_timeout=30000;")
        .context("set busy_timeout=30000")?;
    check_schema(&conn, path)?;
    Ok(Store {
        conn,
        path: path.to_path_buf(),
    })
}

fn check_schema(conn: &Connection, path: &Path) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("read PRAGMA user_version")?;
    if version != SCHEMA_VERSION {
        bail!(
            "opencoder store schema version {version} (expected {SCHEMA_VERSION}) at {}; \
             refusing to write - upgrade terminator-rust or run the opencode migration once",
            path.display()
        );
    }
    check_columns(conn, path, "session_inputs", &REQUIRED_INPUT_COLUMNS)?;
    check_columns(conn, path, "sessions", &REQUIRED_SESSION_COLUMNS)?;
    Ok(())
}

fn check_columns(conn: &Connection, path: &Path, table: &str, required: &[&str]) -> Result<()> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&sql)
        .with_context(|| format!("inspect {table} columns"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("read {table} columns"))?
        .collect::<std::result::Result<Vec<String>, _>>()
        .with_context(|| format!("decode {table} columns"))?;
    let missing: Vec<&str> = required
        .iter()
        .filter(|col| !names.iter().any(|name| name == *col))
        .copied()
        .collect();
    if !missing.is_empty() {
        bail!(
            "opencoder store at {}: {} is missing column(s): {}",
            path.display(),
            table,
            missing.join(", ")
        );
    }
    Ok(())
}

/// A session header as shown in session pickers.
pub struct SessionRow {
    pub id: String,
    pub title: Option<String>,
    pub updated_at: i64,
    pub workdir_hash: Option<String>,
}

/// A not-yet-claimed user input. `created_order` is the table's
/// AUTOINCREMENT `seq`, i.e. global insertion order.
pub struct PendingInput {
    pub seq: i64,
    pub id: String,
    pub delivery: String,
    pub prompt: String,
    pub admitted_seq: i64,
    pub created_order: i64,
}

/// A consumed-input receipt parsed from a `session_events` row.
pub struct Receipt {
    pub seq: i64,
    pub kind: String,
    pub text: String,
    pub ts: i64,
}

impl Store {
    /// Path this store was opened from (for CLI diagnostics).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Sessions, most recently updated first.
    pub fn sessions(&self, limit: u32) -> Result<Vec<SessionRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, title, updated_at, workdir_hash FROM sessions
                 ORDER BY updated_at DESC LIMIT ?1",
            )
            .context("prepare sessions query")?;
        let rows = stmt
            .query_map([limit], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                    workdir_hash: row.get(3)?,
                })
            })
            .context("run sessions query")?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("decode session rows")
    }

    /// The single most recently updated session, if any.
    pub fn latest_session(&self) -> Result<Option<SessionRow>> {
        Ok(self.sessions(1)?.into_iter().next())
    }

    pub fn session_exists(&self, id: &str) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row("SELECT 1 FROM sessions WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .context("check session exists")?;
        Ok(found.is_some())
    }

    /// Append a pending input for the TUI to claim. Returns the new
    /// `session_inputs.seq`. `delivery` must be "steer" or "queue".
    pub fn insert_input(&self, session: &str, delivery: &str, prompt: &str) -> Result<i64> {
        if delivery != "steer" && delivery != "queue" {
            bail!("delivery must be \"steer\" or \"queue\", got {delivery:?}");
        }
        if !self.session_exists(session)? {
            bail!("session {session} not found in this store");
        }
        self.conn
            .execute("BEGIN IMMEDIATE", [])
            .context("BEGIN IMMEDIATE on session_inputs")?;
        match self.insert_input_row(session, delivery, prompt) {
            Ok(seq) => {
                self.conn
                    .execute("COMMIT", [])
                    .context("commit session_inputs insert")?;
                Ok(seq)
            }
            Err(err) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(err)
            }
        }
    }

    fn insert_input_row(&self, session: &str, delivery: &str, prompt: &str) -> Result<i64> {
        let next: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(admitted_seq), 0) + 1 FROM session_inputs
                 WHERE session_id = ?1",
                [session],
                |row| row.get(0),
            )
            .with_context(|| format!("compute next admitted_seq for session {session}"))?;
        self.conn
            .execute(
                "INSERT INTO session_inputs
                   (id, session_id, delivery, prompt, images_json, display_text,
                    admitted_seq, promoted_seq, recorded)
                 VALUES (?1, ?2, ?3, ?4, '[]', NULL, ?5, NULL, 0)",
                rusqlite::params![crate::ulid::new(), session, delivery, prompt, next],
            )
            .with_context(|| format!("insert {delivery} input for session {session}"))?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Inputs not yet claimed by the runner, admission order first.
    pub fn pending(&self, session: &str) -> Result<Vec<PendingInput>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT seq, id, delivery, prompt, admitted_seq FROM session_inputs
                 WHERE session_id = ?1 AND promoted_seq IS NULL AND recorded = 0
                 ORDER BY admitted_seq ASC, seq ASC",
            )
            .context("prepare pending query")?;
        let rows = stmt
            .query_map([session], |row| {
                let seq: i64 = row.get(0)?;
                Ok(PendingInput {
                    seq,
                    id: row.get(1)?,
                    delivery: row.get(2)?,
                    prompt: row.get(3)?,
                    admitted_seq: row.get(4)?,
                    // In this schema creation order is the AUTOINCREMENT seq.
                    created_order: seq,
                })
            })
            .context("run pending query")?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("decode pending rows")
    }

    /// Consumed-input receipts, oldest first (the newest `limit` events).
    pub fn receipts(&self, session: &str, limit: u32) -> Result<Vec<Receipt>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT seq, type, payload_json, ts FROM session_events
                 WHERE session_id = ?1
                   AND type IN ('steer_consumed', 'queue_consumed')
                 ORDER BY seq DESC LIMIT ?2",
            )
            .context("prepare receipts query")?;
        let rows = stmt
            .query_map(rusqlite::params![session, limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .context("run receipts query")?;
        let mut out = Vec::new();
        for row in rows {
            let (eseq, kind, payload_json, ts) =
                row.with_context(|| format!("decode receipt event for session {session}"))?;
            let payload: serde_json::Value = serde_json::from_str(&payload_json)
                .with_context(|| format!("parse payload of event {eseq}"))?;
            // Without the referenced seq the receipt is unusable: skip it.
            let Some(seq) = payload.get("seq").and_then(|v| v.as_i64()) else {
                continue;
            };
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            out.push(Receipt {
                seq,
                kind,
                text,
                ts,
            });
        }
        out.reverse();
        Ok(out)
    }

    /// Raw SQL for tests only (schema mutation / consumption simulation).
    #[cfg(test)]
    pub(crate) fn exec(&self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql).context("test exec")
    }
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod tests;
