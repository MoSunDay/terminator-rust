//! Output shaping: wire structs -> exact bytes printed. Pure, so the CLI's
//! look is unit-testable without a socket.

use std::collections::HashSet;
use std::path::Path;

use ipc_proto::{CaptureOut, PaneInfo, Response};
use serde::Serialize;

const HEADER: [&str; 8] = [
    "ID",
    "WIN",
    "NAME",
    "KIND",
    "COLSxROWS",
    "PID",
    "ALIVE",
    "EXIT",
];

pub fn pane_table(panes: &[PaneInfo]) -> String {
    render_table(&HEADER, &panes.iter().map(row_of).collect::<Vec<_>>())
}

/// Column table shared by `pane_table`/`instances_table`: header row plus
/// data rows, every column padded to its widest cell (by chars).
fn render_table(header: &[&str], data: &[Vec<String>]) -> String {
    let mut rows: Vec<Vec<String>> = vec![header.iter().map(|h| (*h).to_string()).collect()];
    rows.extend(data.iter().cloned());
    let widths: Vec<usize> = (0..header.len())
        // chars, not bytes: `{:<width$}` pads by chars, so a CJK/emoji
        // name (3 bytes/char) would blow the column apart otherwise.
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    let mut out = String::new();
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| format!("{cell:<width$}", width = widths[c]))
            .collect();
        out.push_str(line.join(" ").trim_end());
        out.push('\n');
    }
    out
}

fn row_of(p: &PaneInfo) -> Vec<String> {
    vec![
        p.id.to_string(),
        p.window.to_string(),
        p.name.clone().unwrap_or_else(|| "-".into()),
        p.kind.clone(),
        format!("{}x{}", p.cols, p.rows),
        p.pid.to_string(),
        if p.alive { "yes".into() } else { "no".into() },
        p.exit.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
    ]
}

pub fn pane_json(panes: &[PaneInfo]) -> String {
    serde_json::to_string_pretty(panes).unwrap_or_else(|_| "[]".into())
}

/// One `instances` row: a discovered control socket plus what a live
/// `List` probe reported. Field order fixes the `--json` object order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceRow {
    pub socket: String,
    pub panes: usize,
    /// Distinct pane window ids (0 = unknown, excluded).
    pub windows: usize,
    /// Pid from an `ipc-<pid>.sock` name; None for the primary socket.
    pub pid: Option<u32>,
    pub alive: bool,
}

/// `ipc-<pid>.sock` -> `Some(pid)`; the primary `ipc.sock` and anything
/// unparseable -> `None` (rendered `-` / JSON null).
pub fn pid_from_socket(path: &Path) -> Option<u32> {
    let mid = path
        .file_name()?
        .to_str()?
        .strip_prefix("ipc-")?
        .strip_suffix(".sock")?;
    if !mid.is_empty() && mid.bytes().all(|b| b.is_ascii_digit()) {
        mid.parse().ok()
    } else {
        None
    }
}

/// Build one row from a discovered path and its probe answer (`None` =
/// dead socket: panes/windows read as 0).
pub fn instance_row(path: &Path, answer: Option<&Response>) -> InstanceRow {
    let (panes, windows) = match answer {
        Some(Response::List { panes }) => (
            panes.len(),
            panes
                .iter()
                .map(|p| p.window)
                .filter(|w| *w != 0)
                .collect::<HashSet<_>>()
                .len(),
        ),
        _ => (0, 0),
    };
    InstanceRow {
        socket: path.display().to_string(),
        panes,
        windows,
        pid: pid_from_socket(path),
        alive: answer.is_some(),
    }
}

const INSTANCE_HEADER: [&str; 5] = ["SOCKET", "PANES", "WINDOWS", "PID", "ALIVE"];

pub fn instances_table(rows: &[InstanceRow]) -> String {
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.socket.clone(),
                r.panes.to_string(),
                r.windows.to_string(),
                r.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
                if r.alive { "yes".into() } else { "no".into() },
            ]
        })
        .collect();
    render_table(&INSTANCE_HEADER, &data)
}

pub fn instances_json(rows: &[InstanceRow]) -> String {
    serde_json::to_string_pretty(rows).unwrap_or_else(|_| "[]".into())
}

/// Screen text exactly as captured, guaranteed newline-terminated so shell
/// prompts start on a fresh line.
pub fn capture_text(out: &CaptureOut) -> String {
    let mut t = out.text.clone();
    if !t.ends_with('\n') {
        t.push('\n');
    }
    t
}

pub fn capture_json(out: &CaptureOut) -> String {
    serde_json::to_string_pretty(out).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> PaneInfo {
        PaneInfo {
            id: 7,
            name: Some("agent-one".into()),
            kind: "local".into(),
            osc_title: "zsh".into(),
            cols: 120,
            rows: 40,
            pid: 4242,
            alive: true,
            exit: None,
            window: 2,
        }
    }

    #[test]
    fn table_header_and_columns() {
        let table = pane_table(&[pane()]);
        let mut lines = table.lines();
        let header = lines.next().unwrap();
        // widths: ID 2, WIN 3, NAME 9 (agent-one), KIND 5, COLSxROWS 9,
        // PID 4, ALIVE 5, EXIT 1 (-)
        assert_eq!(header, "ID WIN NAME      KIND  COLSxROWS PID  ALIVE EXIT");
        let row = lines.next().unwrap();
        assert_eq!(row, "7  2   agent-one local 120x40    4242 yes   -");
        assert!(lines.next().is_none());
    }

    #[test]
    fn table_defaults_for_missing_fields() {
        let mut p = pane();
        p.name = None;
        p.alive = false;
        p.exit = Some(127);
        let out = pane_table(&[p]);
        assert!(out.contains(" - "), "name dash: {out}");
        // "no" pads to ALIVE-width 5, then the column gap: 4 spaces to 127.
        assert!(out.contains("no    127"), "alive/exit: {out}");
    }

    #[test]
    fn table_widths_count_chars_not_bytes() {
        let mut cjk = pane();
        cjk.name = Some("汉字测试".into()); // 4 chars, 12 bytes
        let table = pane_table(&[pane(), cjk]);
        let mut lines = table.lines();
        let _header = lines.next().unwrap();
        let ascii_row = lines.next().unwrap();
        let cjk_row = lines.next().unwrap();
        // NAME width = max(4, 9, 4 chars) = 9: the CJK name field pads to
        // 9 chars (4 han + 5 spaces). Byte-width (12) would push its KIND
        // column 3 chars right of the ASCII row's.
        let kind_at = |s: &str| {
            s.match_indices("local")
                .next()
                .map(|(i, _)| s[..i].chars().count())
        };
        assert_eq!(kind_at(ascii_row), kind_at(cjk_row));
        // 4 han chars padded to the 9-char NAME width + 1 column separator.
        assert!(cjk_row.contains("汉字测试      local"), "{cjk_row}");
    }

    #[test]
    fn json_and_capture() {
        let j = pane_json(&[pane()]);
        assert!(j.contains("\"name\": \"agent-one\""), "{j}");
        let mut cap = CaptureOut {
            text: "screen".into(),
            cols: 10,
            rows: 2,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            exit: None,
        };
        assert_eq!(capture_text(&cap), "screen\n");
        cap.text = "already\n".into();
        assert_eq!(capture_text(&cap), "already\n");
        let cj = capture_json(&cap);
        assert!(cj.contains("\"cols\": 10"), "{cj}");
    }

    #[test]
    fn pid_from_socket_names() {
        let p = |n: &str| pid_from_socket(Path::new("/run/x").join(n).as_path());
        assert_eq!(p("ipc-4242.sock"), Some(4242));
        assert_eq!(p("ipc.sock"), None, "primary has no pid");
        assert_eq!(p("ipcfoo.sock"), None, "not ipc-<pid> shaped");
        assert_eq!(p("ipc-abc.sock"), None, "non-numeric middle");
        assert_eq!(p("ipc-.sock"), None, "empty middle");
    }

    #[test]
    fn instance_row_counts_panes_and_windows() {
        let live = Response::List {
            panes: vec![pane(), pane(), {
                let mut other = pane();
                other.id = 9;
                other.window = 3;
                other
            }],
        };
        let row = instance_row(
            Path::new("/run/user/1000/terminator-rust/ipc-4242.sock"),
            Some(&live),
        );
        assert_eq!(row.pid, Some(4242));
        assert_eq!(row.panes, 3);
        assert_eq!(row.windows, 2, "windows 2 and 3, distinct");
        assert!(row.alive);

        // dead answer and window 0 (unknown) exclusion
        let mut stale = pane();
        stale.window = 0;
        let zero = Response::List { panes: vec![stale] };
        let row = instance_row(Path::new("/run/x/ipc.sock"), Some(&zero));
        assert_eq!(row.windows, 0);
        let dead = instance_row(Path::new("/run/x/ipc-7.sock"), None);
        assert_eq!(
            dead,
            InstanceRow {
                socket: "/run/x/ipc-7.sock".into(),
                panes: 0,
                windows: 0,
                pid: Some(7),
                alive: false,
            }
        );
    }

    #[test]
    fn instances_table_and_json_shapes() {
        let rows = [
            InstanceRow {
                socket: "/run/user/1000/terminator-rust/ipc-4242.sock".into(),
                panes: 3,
                windows: 2,
                pid: Some(4242),
                alive: true,
            },
            InstanceRow {
                socket: "/run/user/1000/terminator-rust/ipc.sock".into(),
                panes: 0,
                windows: 0,
                pid: None,
                alive: false,
            },
        ];
        let table = instances_table(&rows);
        let mut lines = table.lines();
        let header = lines.next().unwrap();
        assert_eq!(
            header,
            "SOCKET                                       PANES WINDOWS PID  ALIVE"
        );
        let live = lines.next().unwrap();
        assert_eq!(
            live,
            "/run/user/1000/terminator-rust/ipc-4242.sock 3     2       4242 yes"
        );
        let dead = lines.next().unwrap();
        assert_eq!(
            dead,
            "/run/user/1000/terminator-rust/ipc.sock      0     0       -    no"
        );
        assert!(lines.next().is_none());

        let j = instances_json(&rows);
        assert!(j.contains("\"socket\""), "{j}");
        assert!(j.contains("\"pid\": 4242"), "{j}");
        assert!(j.contains("\"pid\": null"), "{j}");
        assert!(j.contains("\"windows\": 2"), "{j}");
        assert_eq!(instances_json(&[]), "[]");
    }
}
