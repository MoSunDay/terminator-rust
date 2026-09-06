//! Tests for [`crate::db`]. Kept in a sibling file so `db.rs` stays small.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rusqlite::Connection;

use super::{open_ro, open_rw, Store};

/// Exact DDL of the real OpenCoder store (schema generation 18).
const DDL: &str = "
CREATE TABLE IF NOT EXISTS sessions (
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
  autopilot_mode TEXT
);
CREATE TABLE IF NOT EXISTS session_inputs (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  id           TEXT NOT NULL,
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  delivery     TEXT NOT NULL,
  prompt       TEXT NOT NULL,
  images_json  TEXT NOT NULL DEFAULT '[]',
  display_text TEXT,
  admitted_seq INTEGER NOT NULL,
  promoted_seq INTEGER,
  recorded     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS session_events (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  type        TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  sse_kind    TEXT,
  ts          INTEGER NOT NULL
);
PRAGMA user_version=18;
INSERT INTO sessions (id, title, created_at, updated_at, workdir_hash)
  VALUES ('s1', 'first session', 500, 1000, 'hash-1'),
         ('s2', 'second session', 600, 2000, 'hash-2');
";

/// DDL with `session_inputs.recorded` dropped: schema-guard fixture.
const DDL_NO_RECORDED: &str = "
CREATE TABLE sessions (
  id        TEXT PRIMARY KEY,
  title     TEXT,
  updated_at INTEGER NOT NULL
);
CREATE TABLE session_inputs (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  id           TEXT NOT NULL,
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  delivery     TEXT NOT NULL,
  prompt       TEXT NOT NULL,
  images_json  TEXT NOT NULL DEFAULT '[]',
  display_text TEXT,
  admitted_seq INTEGER NOT NULL,
  promoted_seq INTEGER
);
CREATE TABLE session_events (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  type        TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  sse_kind    TEXT,
  ts          INTEGER NOT NULL
);
PRAGMA user_version=18;
";

static DIR_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_root(tag: &str) -> PathBuf {
    let n = DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("oc-store-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir); // best effort, incl. -wal/-shm
}

fn mkdb(dir: &Path) -> PathBuf {
    mkdb_at(&dir.join("store.db"))
}

fn mkdb_at(path: &Path) -> PathBuf {
    let conn = Connection::open(path).expect("create fixture db");
    conn.execute_batch(DDL).expect("build fixture schema");
    path.to_path_buf()
}

fn consume(store: &Store, seq: i64) {
    store
        .exec(&format!(
            "UPDATE session_inputs SET promoted_seq = 1, recorded = 1 WHERE seq = {seq}"
        ))
        .expect("simulate consumption");
}

fn add_event(store: &Store, kind: &str, payload: &str, ts: i64) {
    store
        .exec(&format!(
            "INSERT INTO session_events (session_id, type, payload_json, ts)
             VALUES ('s1', '{kind}', '{payload}', {ts})"
        ))
        .expect("insert event");
}

#[test]
fn insert_input_then_pending_and_consumption() {
    let dir = temp_root("insert");
    let db = mkdb(&dir);
    let store = open_rw(&db).expect("open_rw fixture");

    let seq1 = store
        .insert_input("s1", "steer", "fix the bug")
        .expect("insert 1");
    assert_eq!(seq1, 1);
    let pending = store.pending("s1").expect("pending 1");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].seq, 1);
    assert_eq!(pending[0].delivery, "steer");
    assert_eq!(pending[0].prompt, "fix the bug");
    assert_eq!(pending[0].admitted_seq, 1);
    assert_eq!(pending[0].created_order, 1);
    assert_eq!(pending[0].id.len(), 26);

    let seq2 = store
        .insert_input("s1", "queue", "also update docs")
        .expect("insert 2");
    assert_eq!(seq2, 2);
    let pending = store.pending("s1").expect("pending 2");
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[1].admitted_seq, 2);
    assert_ne!(pending[0].id, pending[1].id);
    assert!(store.pending("s2").expect("pending s2").is_empty());

    consume(&store, 1);
    let pending = store.pending("s1").expect("pending after consume");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].seq, 2);
    cleanup(&dir);
}

#[test]
fn receipts_ascending_and_limited() {
    let dir = temp_root("receipts");
    let db = mkdb(&dir);
    let store = open_rw(&db).expect("open_rw fixture");
    store
        .insert_input("s1", "steer", "fix the bug")
        .expect("insert 1");
    store
        .insert_input("s1", "queue", "also update docs")
        .expect("insert 2");
    add_event(
        &store,
        "steer_consumed",
        r#"{"seq":1,"text":"fix the bug"}"#,
        111,
    );
    add_event(
        &store,
        "queue_consumed",
        r#"{"seq":2,"text":"also update docs"}"#,
        222,
    );
    let receipts = store.receipts("s1", 10).expect("receipts");
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].seq, 1);
    assert_eq!(receipts[0].kind, "steer_consumed");
    assert_eq!(receipts[0].text, "fix the bug");
    assert_eq!(receipts[0].ts, 111);
    assert_eq!(receipts[1].seq, 2);
    assert_eq!(receipts[1].kind, "queue_consumed");
    assert_eq!(receipts[1].ts, 222);

    let one = store.receipts("s1", 1).expect("receipts limit 1");
    assert_eq!(one.len(), 1);
    // The window is the newest `limit` events (DESC query), so this is the
    // most recent receipt, returned ascending.
    assert_eq!(one[0].seq, 2);

    // Unusable receipt (no seq) must be skipped, not fail the query.
    add_event(&store, "steer_consumed", r#"{"text":"no seq here"}"#, 333);
    let receipts = store.receipts("s1", 10).expect("receipts after junk");
    assert_eq!(receipts.len(), 2);
    assert!(store.receipts("s2", 10).expect("receipts s2").is_empty());
    cleanup(&dir);
}

#[test]
fn guards_reject_foreign_schemas_and_bad_requests() {
    let dir = temp_root("guards");
    let db = mkdb(&dir);

    {
        let conn = Connection::open(&db).expect("reopen fixture");
        conn.execute_batch("PRAGMA user_version=17;")
            .expect("downgrade");
    }
    let err = open_rw(&db).expect_err("open downgraded db").to_string();
    assert!(err.contains("expected 18"), "{err}");
    let err = open_ro(&db).expect_err("open ro downgraded db").to_string();
    assert!(err.contains("expected 18"), "{err}");

    let bad = dir.join("bad.db");
    {
        let conn = Connection::open(&bad).expect("create bad db");
        conn.execute_batch(DDL_NO_RECORDED).expect("bad schema");
    }
    let err = open_rw(&bad).expect_err("open bad db").to_string();
    assert!(err.contains("recorded"), "{err}");

    let err = open_rw(&dir.join("missing.db"))
        .expect_err("open missing db")
        .to_string();
    assert!(err.contains("no opencoder store"), "{err}");

    let db2 = mkdb_at(&dir.join("store2.db"));
    let store = open_rw(&db2).expect("open good fixture");
    let err = store
        .insert_input("ghost", "steer", "hi")
        .expect_err("unknown session")
        .to_string();
    assert!(err.contains("session ghost not found"), "{err}");
    let err = store
        .insert_input("s1", "urgent", "hi")
        .expect_err("bad delivery")
        .to_string();
    assert!(err.contains("steer"), "{err}");
    // Failed inserts must roll back cleanly.
    assert!(store.pending("s1").expect("pending").is_empty());
    cleanup(&dir);
}

#[test]
fn open_ro_reads_fixture() {
    let dir = temp_root("readonly");
    let db = mkdb(&dir);
    let store = open_rw(&db).expect("open_rw to seed");
    store
        .insert_input("s1", "queue", "later")
        .expect("seed input");
    consume(&store, 1);
    add_event(&store, "queue_consumed", r#"{"seq":1,"text":"later"}"#, 55);
    drop(store);

    let ro = open_ro(&db).expect("open_ro fixture");
    let sessions = ro.sessions(10).expect("sessions");
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "s2"); // updated_at DESC
    assert_eq!(sessions[0].updated_at, 2000);
    assert_eq!(ro.latest_session().expect("latest").unwrap().id, "s2");
    assert!(ro.session_exists("s1").expect("exists s1"));
    assert!(!ro.session_exists("s3").expect("exists s3"));
    assert!(ro.pending("s1").expect("pending ro").is_empty());
    let receipts = ro.receipts("s1", 10).expect("receipts ro");
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].text, "later");
    assert!(ro.insert_input("s1", "steer", "nope").is_err());
    cleanup(&dir);
}
