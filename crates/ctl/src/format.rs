//! Output shaping: wire structs -> exact bytes printed. Pure, so the CLI's
//! look is unit-testable without a socket.

use ipc_proto::{CaptureOut, PaneInfo};

const HEADER: [&str; 7] = ["ID", "NAME", "KIND", "COLSxROWS", "PID", "ALIVE", "EXIT"];

pub fn pane_table(panes: &[PaneInfo]) -> String {
    let mut rows: Vec<Vec<String>> = vec![HEADER.iter().map(|h| (*h).to_string()).collect()];
    rows.extend(panes.iter().map(row_of));
    let widths: Vec<usize> = (0..HEADER.len())
        .map(|c| rows.iter().map(|r| r[c].len()).max().unwrap_or(0))
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
        }
    }

    #[test]
    fn table_header_and_columns() {
        let table = pane_table(&[pane()]);
        let mut lines = table.lines();
        let header = lines.next().unwrap();
        // widths: ID 2, NAME 9 (agent-one), KIND 5, COLSxROWS 9, PID 4,
        // ALIVE 5, EXIT 1 (-)
        assert_eq!(header, "ID NAME      KIND  COLSxROWS PID  ALIVE EXIT");
        let row = lines.next().unwrap();
        assert_eq!(row, "7  agent-one local 120x40    4242 yes   -");
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
}
