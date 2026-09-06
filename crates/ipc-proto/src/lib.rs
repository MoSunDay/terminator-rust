//! Wire types shared by the terminator-rust app's UDS control socket and
//! external clients (`terminator-ctl`). Serde-only: no app, UI or pty types,
//! so both sides depend on this crate without dragging the rest in.
//!
//! Wire shape: one JSON object per line, requests tagged with `"cmd"`:
//! `{"cmd":"capture","pane":"agent1","lines":80}`. Pane selectors are
//! untagged: a JSON string addresses a pane by its (unique) manual title,
//! a JSON number by pane id.

use serde::{Deserialize, Serialize};

/// Env var carrying the control socket path to pane children.
pub const ENV_SOCKET: &str = "TERMINATOR_SOCK";
/// Default connect/read timeout for clients, in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 5;

/// Address a pane either by unique manual title or by pane id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PaneSelector {
    Name(String),
    Id(u64),
}

impl PaneSelector {
    pub fn name(&self) -> Option<&str> {
        match self {
            PaneSelector::Name(n) => Some(n.as_str()),
            PaneSelector::Id(_) => None,
        }
    }
}

/// One pane as seen over the socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: u64,
    /// Unique manual title, when set (the addressing key for `oc`/`send`).
    pub name: Option<String>,
    /// "local" or "remote" (remote panes carry the host label here).
    pub kind: String,
    /// OSC 0/2 title reported by the program, "" when absent.
    pub osc_title: String,
    pub cols: u16,
    pub rows: u16,
    /// pid of the pane's direct child, 0 when no live session.
    pub pid: i32,
    pub alive: bool,
    /// Observed exit code once the child terminated.
    pub exit: Option<i32>,
}

/// Screen capture result: plain text plus enough geometry to interpret it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureOut {
    /// Visible screen rows, joined with '\n', trailing blanks trimmed per
    /// line; at most `lines` tail rows.
    pub text: String,
    pub cols: u16,
    pub rows: u16,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub exit: Option<i32>,
}

/// Client -> app request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    List,
    Capture {
        pane: PaneSelector,
        #[serde(default = "default_lines")]
        lines: u32,
    },
    Write {
        pane: PaneSelector,
        text: String,
        /// false = raw bytes (escape hatch), true = bracketed-paste aware.
        #[serde(default)]
        bracketed: bool,
    },
}

fn default_lines() -> u32 {
    80
}

/// App -> client response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ok", rename_all = "snake_case")]
pub enum Response {
    #[serde(rename = "list")]
    List { panes: Vec<PaneInfo> },
    #[serde(rename = "capture")]
    Capture(CaptureOut),
    #[serde(rename = "written")]
    Written { bytes: usize },
    #[serde(rename = "error")]
    Error { message: String },
}

impl Response {
    /// Human-readable one-liner for CLI display; errors get "error: " prefix.
    pub fn err(message: impl Into<String>) -> Self {
        Response::Error {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let req: Request = serde_json::from_str(r#"{"cmd":"capture","pane":"agent1"}"#).unwrap();
        assert_eq!(
            req,
            Request::Capture {
                pane: PaneSelector::Name("agent1".into()),
                lines: 80
            }
        );
        let by_id: Request =
            serde_json::from_str(r#"{"cmd":"capture","pane":7,"lines":10}"#).unwrap();
        assert_eq!(
            by_id,
            Request::Capture {
                pane: PaneSelector::Id(7),
                lines: 10
            }
        );
        let out = serde_json::to_string(&by_id).unwrap();
        assert!(
            out.contains(r#""pane":7"#) && out.contains(r#""lines":10"#),
            "{out}"
        );
    }

    #[test]
    fn write_and_list_roundtrip() {
        let w: Request =
            serde_json::from_str(r#"{"cmd":"write","pane":"a","text":"hi\n"}"#).unwrap();
        assert_eq!(
            w,
            Request::Write {
                pane: PaneSelector::Name("a".into()),
                text: "hi\n".into(),
                bracketed: false
            }
        );
        let l: Request = serde_json::from_str(r#"{"cmd":"list"}"#).unwrap();
        assert_eq!(l, Request::List);
        assert_eq!(
            serde_json::to_string(&Response::err("boom")).unwrap(),
            r#"{"ok":"error","message":"boom"}"#
        );
        let r: Response = serde_json::from_str(r#"{"ok":"written","bytes":3}"#).unwrap();
        assert_eq!(r, Response::Written { bytes: 3 });
    }
}
