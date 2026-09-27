use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Socket commands: A attaches, Q queries, I carries input, R resizes.
pub const ATTACH: u8 = b'A';
pub const QUERY: u8 = b'Q';
pub const INPUT: u8 = b'I';
pub const RESIZE: u8 = b'R';
pub const MAX_INPUT: usize = 8192;
pub const HISTORY_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct Summary {
    pub name: String,
    pub title: String,
    pub cwd: String,
    pub attached: bool,
}

pub fn input_packet(bytes: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(5 + bytes.len());
    packet.push(INPUT);
    packet.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    packet.extend_from_slice(bytes);
    packet
}

pub fn resize_packet(cols: u16, rows: u16) -> [u8; 5] {
    let [ch, cl] = cols.to_be_bytes();
    let [rh, rl] = rows.to_be_bytes();
    [RESIZE, ch, cl, rh, rl]
}

pub fn push_history(history: &mut VecDeque<u8>, bytes: &[u8]) {
    history.extend(bytes.iter().copied());
    while history.len() > HISTORY_LIMIT {
        history.pop_front();
    }
}

pub fn list_line(s: &Summary) -> String {
    let state = if s.attached { "attached" } else { "detached" };
    let title = s.title.replace(['\n', '\r', '\t'], " ");
    let cwd = s.cwd.replace(['\n', '\r', '\t'], " ");
    let cwd = if cwd.chars().count() > 36 {
        format!(
            "…{}",
            cwd.chars()
                .rev()
                .take(35)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )
    } else {
        cwd
    };
    format!("{:<24} {:<10} {:<36} {}", s.name, state, cwd, title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_exposes_purpose_and_state() {
        let s = Summary {
            name: "build".into(),
            title: "release build".into(),
            cwd: "/work/app".into(),
            attached: false,
        };
        let line = list_line(&s);
        assert!(line.contains("build") && line.contains("detached"));
        assert!(line.contains("/work/app") && line.contains("release build"));
    }
}
