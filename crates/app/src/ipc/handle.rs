//! Request handling: translate wire requests into actions on live state.
//!
//! Everything here runs on the UI thread (called from
//! [`super::server::drain`]) and therefore touches sessions directly.

use ipc_proto::{CaptureOut, PaneInfo, PaneSelector, Request, Response};
use layout_tree::PaneId;
use log::warn;
use remote::PaneKind;
use vt_pane::task as vtask;

use crate::session_map::SessionMap;
use crate::state::{AppState, Data};

/// Dispatch one request against the live app state.
pub fn execute(req: Request, data: &mut Data) -> Response {
    match req {
        Request::List => list(&data.st, &mut data.sess),
        Request::Capture { pane, lines } => match resolve(&data.st, &pane) {
            Ok(id) => match data.sess.map.get_mut(&id) {
                Some(sess) => capture(sess, lines),
                None => Response::err(format!("pane {id} has no live session")),
            },
            Err(resp) => resp,
        },
        Request::Write {
            pane,
            text,
            bracketed,
        } => match resolve(&data.st, &pane) {
            Ok(id) => match data.sess.map.get_mut(&id) {
                Some(sess) => write(sess, &text, bracketed),
                None => Response::err(format!("pane {id} has no live session")),
            },
            Err(resp) => resp,
        },
    }
}

/// All panes in display order with live-session facts folded in; panes
/// without a session are still listed (zeros / empty fields).
fn list(st: &AppState, sess: &mut SessionMap) -> Response {
    let mut ids: Vec<PaneId> = Vec::new();
    for tab in &st.tree.tabs {
        layout_tree::pane_ids(&tab.root, &mut ids);
    }
    ids.dedup();
    let panes = ids
        .iter()
        .filter_map(|id| {
            let meta = st.panes.get(id)?;
            let kind = match &meta.kind {
                PaneKind::Local => "local".to_string(),
                PaneKind::Remote(t) => format!("remote:{}", t.host),
            };
            let mut info = PaneInfo {
                id: *id,
                name: meta.manual_title.clone(),
                kind,
                osc_title: String::new(),
                cols: 0,
                rows: 0,
                pid: 0,
                alive: false,
                exit: None,
            };
            if let Some(s) = sess.map.get_mut(id) {
                info.osc_title = vtask::title(s);
                info.pid = vtask::child_pid(s);
                info.exit = s.exit;
                info.alive = s.exit.is_none();
                if let Ok(f) = vtask::frame(s) {
                    info.cols = f.cols;
                    info.rows = f.rows;
                }
            }
            Some(info)
        })
        .collect();
    Response::List { panes }
}

/// Pump the session, snapshot the frame and flatten it to text.
fn capture(sess: &mut vt_pane::Session, lines: u32) -> Response {
    if let Err(e) = vtask::pump(sess) {
        warn!("ipc pump: {e}");
    }
    match vtask::frame(sess) {
        Ok(frame) => Response::Capture(CaptureOut {
            text: frame_text(&frame, lines),
            cols: frame.cols,
            rows: frame.rows,
            cursor_x: frame.cursor.x,
            cursor_y: frame.cursor.y,
            cursor_visible: frame.cursor.visible,
            exit: sess.exit,
        }),
        Err(e) => Response::err(format!("capture: {e}")),
    }
}

/// Raw or bracketed-paste-aware input injection.
fn write(sess: &mut vt_pane::Session, text: &str, bracketed: bool) -> Response {
    let result = if bracketed {
        vtask::paste(sess, text)
    } else {
        vtask::write(sess, text.as_bytes())
    };
    match result {
        Ok(()) => Response::Written { bytes: text.len() },
        Err(e) => Response::err(format!("write: {e}")),
    }
}

/// Visible screen as text: every row in order, each right-trimmed, keeping
/// only the last `lines` rows (0 -> empty string). Interior blank lines
/// survive; only trailing rows are dropped.
pub fn frame_text(frame: &vt_pane::Frame, lines: u32) -> String {
    if lines == 0 {
        return String::new();
    }
    let rows: Vec<String> = frame
        .cells
        .iter()
        .map(|row| {
            let line: String = row.iter().map(|c| c.text.as_str()).collect();
            line.trim_end().to_string()
        })
        .collect();
    let keep = (lines as usize).min(rows.len());
    rows[rows.len() - keep..].join("\n")
}

/// Map a selector to a pane id. Ids must exist in `st.panes` (membership in
/// a tab is not required); names must match exactly one pane.
fn resolve(st: &AppState, sel: &PaneSelector) -> Result<PaneId, Response> {
    match sel {
        PaneSelector::Id(id) => {
            if st.panes.contains_key(id) {
                Ok(*id)
            } else {
                Err(Response::err(format!("no pane with id {id}")))
            }
        }
        PaneSelector::Name(name) => {
            let hits: Vec<PaneId> = st
                .panes
                .iter()
                .filter(|(_, m)| m.manual_title.as_deref() == Some(name.as_str()))
                .map(|(id, _)| *id)
                .collect();
            match hits.as_slice() {
                [only] => Ok(*only),
                [] => Err(Response::err(format!(
                    "no pane named \"{name}\" (set one by double-clicking a pane title)"
                ))),
                many => Err(Response::err(format!(
                    "name \"{name}\" is ambiguous: panes {many:?}"
                ))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{data, fresh_state};
    use std::thread::sleep;
    use std::time::Duration;
    use vt_pane::{CellData, SessionOpts};

    fn cell(t: &str) -> CellData {
        CellData {
            text: t.to_string(),
            ..Default::default()
        }
    }

    fn frame(rows: &[&str]) -> vt_pane::Frame {
        let mut f = vt_pane::Frame::default();
        for row in rows {
            f.cells
                .push(row.chars().map(|c| cell(&c.to_string())).collect());
        }
        f
    }

    #[test]
    fn frame_text_trims_tails_and_keeps_blanks() {
        let f = frame(&["  hi  ", "", "yo"]);
        assert_eq!(frame_text(&f, 10), "  hi\n\nyo");
        assert_eq!(frame_text(&f, 3), "  hi\n\nyo");
        assert_eq!(frame_text(&f, 2), "\nyo");
        assert_eq!(frame_text(&f, 1), "yo");
        assert_eq!(frame_text(&f, 0), "");
        // All-blank screen: rows survive as empty lines, not dropped.
        let blank = frame(&["", "  ", ""]);
        assert_eq!(frame_text(&blank, 10), "\n\n");
        assert_eq!(frame_text(&blank, 1), "");
    }

    #[test]
    fn frame_text_joins_wide_char_spacers_invisibly() {
        // A CJK glyph spans two cells: the leading cell carries the char,
        // the trailing half is an EMPTY spacer (term.rs cell_data emits no
        // grapheme there). Flattening must not grow a phantom space
        // (interior spacer) nor drop the glyph (line-end spacer).
        let row = |cells: &[&str]| -> Vec<CellData> { cells.iter().map(|t| cell(t)).collect() };
        let mut f = vt_pane::Frame::default();
        f.cells.push(row(&["a", "\u{6c49}", "", "b"])); // a 汉 b
        assert_eq!(frame_text(&f, 5), "a\u{6c49}b");
        f.cells[0] = row(&["\u{6c49}", ""]); // wide glyph at line end
        assert_eq!(frame_text(&f, 5), "\u{6c49}");
        f.cells[0] = row(&["x", "\u{5b57}", "", "  "]);
        assert_eq!(frame_text(&f, 5), "x\u{5b57}", "EOL spacers/spaces trim");
        // two wide glyphs back to back: 4 cells -> 2 chars
        f.cells[0] = row(&["\u{6c49}", "", "\u{5b57}", ""]);
        assert_eq!(frame_text(&f, 5), "\u{6c49}\u{5b57}");
    }

    #[test]
    fn capture_roundtrips_cjk_over_a_real_pty() {
        let opts = SessionOpts {
            cols: 80,
            rows: 6,
            argv: vec!["cat".into()],
            env: vec![],
            scrollback_lines: 100,
        };
        let sess = match vtask::spawn_session(&opts) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let mut d = data(fresh_state(), vec![]);
        d.sess.map.insert(1, sess);
        let _ = execute(
            Request::Write {
                pane: PaneSelector::Id(1),
                text: "\u{6c49}\u{5b57}\n".into(),
                bracketed: false,
            },
            &mut d,
        );
        // cat echoes the bytes; poll until the frame shows them, then pin
        // the exact line: one leading cell + one empty spacer per glyph.
        let mut text = String::new();
        for _ in 0..200 {
            match execute(
                Request::Capture {
                    pane: PaneSelector::Id(1),
                    lines: 6,
                },
                &mut d,
            ) {
                Response::Capture(out) => {
                    text = out.text;
                    if text.contains("\u{6c49}") {
                        break;
                    }
                }
                other => panic!("capture: {other:?}"),
            }
            sleep(Duration::from_millis(10));
        }
        assert!(
            text.lines().any(|l| l.trim_end() == "\u{6c49}\u{5b57}"),
            "exact CJK line missing: {text:?}"
        );
        let mut s = d.sess.map.remove(&1).unwrap();
        vtask::terminate(&mut s);
    }

    #[test]
    fn write_capture_list_roundtrip() {
        let opts = SessionOpts {
            cols: 80,
            rows: 10,
            argv: vec!["cat".into()],
            env: vec![],
            scrollback_lines: 100,
        };
        let sess = match vtask::spawn_session(&opts) {
            Ok(s) => s,
            Err(e) => {
                // CI without a usable pty: skip rather than fail.
                eprintln!("skip: {e}");
                return;
            }
        };
        let mut d = data(fresh_state(), vec![]);
        d.st.panes.get_mut(&1).unwrap().manual_title = Some("t1".into());
        d.sess.map.insert(1, sess);

        let w = execute(
            Request::Write {
                pane: PaneSelector::Name("t1".into()),
                text: "hello\n".into(),
                bracketed: false,
            },
            &mut d,
        );
        assert_eq!(w, Response::Written { bytes: 6 });

        // cat echoes the input back; poll until the frame shows it.
        let mut text = String::new();
        for _ in 0..200 {
            match execute(
                Request::Capture {
                    pane: PaneSelector::Id(1),
                    lines: 10,
                },
                &mut d,
            ) {
                Response::Capture(out) => {
                    assert_eq!((out.cols, out.rows), (80, 10));
                    text = out.text;
                    if text.contains("hello") {
                        break;
                    }
                }
                other => panic!("capture: {other:?}"),
            }
            sleep(Duration::from_millis(10));
        }
        assert!(text.contains("hello"), "capture text: {text:?}");

        match execute(Request::List, &mut d) {
            Response::List { panes } => {
                let p = panes.iter().find(|p| p.id == 1).unwrap();
                assert_eq!(p.name.as_deref(), Some("t1"));
                assert_eq!(p.kind, "local");
                assert_ne!(p.pid, 0);
                assert!(p.alive);
                assert_eq!(p.exit, None);
            }
            other => panic!("list: {other:?}"),
        }

        let bad = execute(
            Request::Write {
                pane: PaneSelector::Name("nope".into()),
                text: "x".into(),
                bracketed: false,
            },
            &mut d,
        );
        assert!(
            matches!(bad, Response::Error { .. }),
            "unknown name must error: {bad:?}"
        );

        let mut s = d.sess.map.remove(&1).unwrap();
        vtask::terminate(&mut s);
    }
}
