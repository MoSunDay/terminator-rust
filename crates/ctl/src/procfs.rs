//! Process-tree walking for the `oc` subcommands: given a pane's pid, find
//! the opencoder process in its tree plus the sqlite db it holds open.
//! Platform-neutral: the per-OS kernel APIs live in `procfs_linux` /
//! `procfs_macos` behind the `platform` alias (pid table, fd paths, cwd);
//! everything here is tree logic shared by both. Forgiving by design: any
//! per-process hiccup (exit mid-scan, hidden fd, EPERM) is skipped
//! silently. The one thing never guessed: which store to use when a
//! process holds several open (see [`pick_db`]).

// pane_pid is kept for future subcommands.
#![allow(dead_code)]

#[cfg(target_os = "linux")]
use crate::procfs_linux as platform;
#[cfg(target_os = "macos")]
use crate::procfs_macos as platform;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use ipc_proto::PaneInfo;

/// One live process row from the platform pid table: pid, parent pid, and
/// the executable's short name (comm). Pure data record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: i32,
    pub ppid: i32,
    pub comm: String,
}

/// An opencoder process found under a pane, with its live state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcProc {
    pub pid: i32,
    pub db: PathBuf,
    pub cwd: PathBuf,
}

/// pid of the pane's direct child (0 when no live session) — trivial
/// accessor kept so call sites read as intent, not field access.
pub fn pane_pid(info: &PaneInfo) -> i32 {
    info.pid
}

/// `root_pid` plus every live descendant, BFS order.
pub fn descendants(root_pid: i32) -> Vec<i32> {
    if root_pid <= 0 {
        return Vec::new();
    }
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for row in platform::proc_table() {
        children.entry(row.ppid).or_default().push(row.pid);
    }
    let mut out = vec![root_pid];
    let mut queue = VecDeque::from([root_pid]);
    while let Some(pid) = queue.pop_front() {
        for child in children.get(&pid).into_iter().flatten() {
            out.push(*child);
            queue.push_back(*child);
        }
    }
    out
}

/// First opencoder process under `root_pid` (inclusive) that has an
/// `opencoder.db` file descriptor open. Err when a candidate process
/// holds several distinct stores open: we cannot know which one its TUI
/// is actually driving, and linking the wrong store would submit prompts
/// into the wrong workdir.
pub fn find_opencoder(root_pid: i32) -> Result<Option<OcProc>, String> {
    let comms: HashMap<i32, String> = platform::proc_table()
        .into_iter()
        .map(|row| (row.pid, row.comm))
        .collect();
    for pid in descendants(root_pid) {
        // exact "opencoder" or a wrapper like "opencoder-tui"
        if !comms
            .get(&pid)
            .is_some_and(|comm| comm.starts_with("opencoder"))
        {
            continue;
        }
        let db = match platform::find_db(pid)? {
            Some(db) => db,
            None => continue,
        };
        let cwd = platform::cwd_of(pid);
        return Ok(Some(OcProc { pid, db, cwd }));
    }
    Ok(None)
}

/// Reduce one process's db-holding fds to the single store it uses.
/// Repeated fds of the SAME file (e.g. one read-write + one read-only)
/// collapse; two distinct paths are an explicit error naming both.
/// Shared by both platform backends' `find_db`.
pub(crate) fn pick_db(paths: Vec<PathBuf>) -> Result<Option<PathBuf>, String> {
    let mut distinct = paths;
    distinct.sort();
    distinct.dedup();
    match distinct.len() {
        0 => Ok(None),
        1 => Ok(distinct.pop()),
        _ => Err(format!(
            "opencoder has multiple stores open ({}); close one, or `oc link <pane> <session-id>` to disambiguate",
            distinct
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_and_find_reject_bad_roots() {
        assert!(descendants(0).is_empty());
        assert!(descendants(-5).is_empty());
        assert_eq!(find_opencoder(0), Ok(None));
        assert_eq!(find_opencoder(-5), Ok(None));
    }

    fn pb(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn pick_db_requires_a_single_distinct_store() {
        assert_eq!(pick_db(vec![]), Ok(None));
        assert_eq!(
            pick_db(vec![pb("/a/opencoder.db")]),
            Ok(Some(pb("/a/opencoder.db")))
        );
        // same file held via two fds (rw + ro) is still one store
        assert_eq!(
            pick_db(vec![pb("/a/opencoder.db"), pb("/a/opencoder.db")]),
            Ok(Some(pb("/a/opencoder.db")))
        );
        let err = pick_db(vec![pb("/b/opencoder.db"), pb("/a/opencoder.db")]).unwrap_err();
        assert!(
            err.contains("/a/opencoder.db") && err.contains("/b/opencoder.db"),
            "{err}"
        );
    }
}
