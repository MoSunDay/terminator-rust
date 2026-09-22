//! Output shaping: wire structs -> exact bytes printed. Pure, so the CLI's
//! look is unit-testable without a socket.

use ipc_proto::{CaptureOut, PaneInfo};

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
    let mut rows: Vec<Vec<String>> = vec![HEADER.iter().map(|h| (*h).to_string()).collect()];
    rows.extend(panes.iter().map(row_of));
    let widths: Vec<usize> = (0..HEADER.len())
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
}
