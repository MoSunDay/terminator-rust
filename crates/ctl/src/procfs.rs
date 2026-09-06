//! /proc walking for the upcoming `oc` subcommands: given a pane's pid,
//! find the opencoder process in its tree plus the sqlite db it holds open.
//! Std-only and forgiving: any IO hiccup (process exited mid-scan, hidden
//! fd) is skipped silently.

// Not wired into a subcommand yet; `cmd_oc` starts here.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use ipc_proto::PaneInfo;

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
    for pid in numeric_pids() {
        if let Some(ppid) = stat_of(pid).as_deref().and_then(ppid_of) {
            children.entry(ppid).or_default().push(pid);
        }
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
/// `opencoder.db` file descriptor open.
pub fn find_opencoder(root_pid: i32) -> Option<OcProc> {
    descendants(root_pid).into_iter().find_map(|pid| {
        let comm = read_comm(pid)?;
        // exact "opencoder" or a wrapper like "opencoder-tui"
        if !comm.starts_with("opencoder") {
            return None;
        }
        let db = find_db(pid)?;
        let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap_or_default();
        Some(OcProc { pid, db, cwd })
    })
}

fn numeric_pids() -> Vec<i32> {
    let mut pids: Vec<i32> = std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| name.parse().ok())
        .collect();
    pids.sort_unstable();
    pids
}

fn stat_of(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
}

/// comm from /proc/pid/comm: trimmed and NUL-safe (the kernel pads it).
fn read_comm(pid: i32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/comm")).ok()?;
    let comm = String::from_utf8_lossy(&raw)
        .trim()
        .trim_end_matches('\0')
        .to_string();
    Some(comm)
}

/// First open fd whose link path ends with "opencoder.db" (sorted for
/// determinism; readdir order is arbitrary).
fn find_db(pid: i32) -> Option<PathBuf> {
    let mut links: Vec<PathBuf> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .filter(|p| p.to_string_lossy().ends_with("opencoder.db"))
        .collect();
    links.sort();
    links.into_iter().next()
}

/// Parse the ppid out of one /proc/pid/stat line. The comm field may
/// contain spaces and ')', so everything before the LAST ')' is comm and
/// the fields after it are: state, ppid, ...
fn ppid_of(stat_line: &str) -> Option<i32> {
    let tail = stat_line.rsplit_once(')').map(|(_, tail)| tail)?;
    let mut fields = tail.split_whitespace();
    fields.next()?; // state
    fields.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ppid_of_handles_parens_in_comm() {
        // comm "some (name)" — only the last ')' closes the field
        let line = "12345 (some (name)) S 999 12345 12345 0 -1 4194560";
        assert_eq!(ppid_of(line), Some(999));
    }

    #[test]
    fn ppid_of_normal_line() {
        let line = "42 (sleep) S 1 42 42 0 -1 11520";
        assert_eq!(ppid_of(line), Some(1));
    }

    #[test]
    fn ppid_of_garbage() {
        assert_eq!(ppid_of(""), None);
        assert_eq!(ppid_of("no parens here"), None);
        assert_eq!(ppid_of(") S notanumber 1"), None);
        assert_eq!(ppid_of(") "), None);
    }

    #[test]
    fn descendants_and_find_reject_bad_roots() {
        assert!(descendants(0).is_empty());
        assert!(descendants(-5).is_empty());
        assert_eq!(find_opencoder(0), None);
        assert_eq!(find_opencoder(-5), None);
    }

    #[test]
    fn live_process_tree_walk() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        assert!(pid > 0);
        let tree = descendants(pid);
        assert_eq!(tree.first(), Some(&pid), "root included: {tree:?}");
        // sleep has no children and no opencoder.db fd
        assert_eq!(find_opencoder(pid), None);
        let mut info = ipc_proto::PaneInfo {
            id: 1,
            name: None,
            kind: "local".into(),
            osc_title: String::new(),
            cols: 80,
            rows: 24,
            pid,
            alive: true,
            exit: None,
        };
        assert_eq!(pane_pid(&info), pid);
        info.pid = 0;
        assert_eq!(pane_pid(&info), 0);
        let _ = child.kill();
        let _ = child.wait();
    }
}
