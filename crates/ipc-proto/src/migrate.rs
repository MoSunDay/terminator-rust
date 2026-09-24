//! Wire types for cross-process tab migration between two terminator-rust
//! instances. The source instance sends a [`super::Request::TabOffer`] JSON
//! header line over the target's control socket, followed by one SCM_RIGHTS
//! fd per pane leaf (depth-first order) and a length-prefixed payload block
//! per leaf carrying its vt snapshot. The receiver assigns fresh pane ids
//! (the tree carries no ids on the wire) and rebuilds the sessions.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// One pane leaf of a migrated tab. `kind` mirrors `PaneInfo::kind`
/// ("local" | "remote:<host>").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigratePane {
    pub manual_title: Option<String>,
    pub kind: String,
    pub degraded: bool,
    /// Child pid (foreign to the receiver; `kill(-pid)` still valid).
    pub pid: i32,
    /// Best-effort; the receiver's sync_frame corrects.
    pub cols: u16,
    pub rows: u16,
}

/// Split axis serialization: "h" | "v". Wire tags are snake_case
/// ("pane"/"split"), matching the request/response enums.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrateNode {
    Pane {
        pane: MigratePane,
    },
    Split {
        axis: String,
        ratio: f32,
        first: Box<MigrateNode>,
        second: Box<MigrateNode>,
    },
}

/// Tab header sent as the JSON line; pane ids are assigned fresh by the
/// receiver (remapped), leaves are in depth-first order matching the
/// SCM_RIGHTS fd array and the payload snapshot blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrateTab {
    pub title: String,
    /// Sender pane id of the focused leaf; receiver remaps, drops if absent.
    pub focused: Option<u64>,
    pub root: MigrateNode,
}

/// Max panes (leaves) per migrated tab; offers beyond this are rejected.
pub const MAX_MIGRATE_PANES: usize = 16;
/// Max total snapshot payload bytes accepted on the wire.
pub const MAX_MIGRATE_PAYLOAD: usize = 32 * 1024 * 1024;

/// Count pane leaves under `node` (depth-first order).
pub fn leaf_count(node: &MigrateNode) -> usize {
    match node {
        MigrateNode::Pane { .. } => 1,
        MigrateNode::Split { first, second, .. } => leaf_count(first) + leaf_count(second),
    }
}

/// Encode per-pane snapshot blocks (leaf order): each block = 4-byte LE
/// length + bytes. A `None` snapshot encodes as a zero-length block (so a
/// `Some(vec![])` snapshot is indistinguishable from `None` on the wire).
pub fn encode_payload(snaps: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut out = Vec::new();
    for snap in snaps {
        let len = snap.as_ref().map_or(0, |b| b.len());
        out.extend_from_slice(&(len as u32).to_le_bytes());
        if let Some(bytes) = snap {
            out.extend_from_slice(bytes);
        }
    }
    out
}

/// Decode `n` blocks; errors (via `String`) on truncation, trailing bytes
/// after the `n`th block, or total > [`MAX_MIGRATE_PAYLOAD`].
pub fn decode_payload(payload: &[u8], n: usize) -> Result<Vec<Option<Vec<u8>>>, String> {
    if payload.len() > MAX_MIGRATE_PAYLOAD {
        return Err(format!(
            "migrate payload {} bytes exceeds limit {}",
            payload.len(),
            MAX_MIGRATE_PAYLOAD
        ));
    }
    let mut out = Vec::with_capacity(n);
    let mut rest = payload;
    for i in 0..n {
        if rest.len() < 4 {
            return Err(format!(
                "migrate payload truncated: block {i} of {n} missing length prefix"
            ));
        }
        let (head, tail) = rest.split_at(4);
        let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        if tail.len() < len {
            return Err(format!(
                "migrate payload truncated: block {i} of {n} wants {len} bytes, has {}",
                tail.len()
            ));
        }
        let (block, tail) = tail.split_at(len);
        out.push(if len == 0 { None } else { Some(block.to_vec()) });
        rest = tail;
    }
    if !rest.is_empty() {
        return Err(format!(
            "migrate payload has {} trailing bytes after {n} blocks",
            rest.len()
        ));
    }
    Ok(out)
}

/// List sibling instance sockets in `dir` (files named `ipc*.sock`),
/// excluding `skip` (own path) if given, sorted. Pure std (`fs::read_dir`),
/// no liveness probing.
pub fn discover_sockets(dir: &Path, skip: Option<&Path>) -> Vec<PathBuf> {
    let skip_key = skip.map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("ipc") || !name.ends_with(".sock") {
            continue;
        }
        if let Some(skip) = &skip_key {
            let cand = path.canonicalize().unwrap_or_else(|_| path.clone());
            if &path == skip || &cand == skip {
                continue;
            }
        }
        out.push(path);
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PaneSelector, Request, Response};

    fn pane(kind: &str) -> MigratePane {
        MigratePane {
            manual_title: None,
            kind: kind.to_string(),
            degraded: false,
            pid: 4242,
            cols: 80,
            rows: 24,
        }
    }

    fn nested_tab() -> MigrateTab {
        // split(h, pane(local), split(v, pane(remote:h1), pane(local)))
        MigrateTab {
            title: "work".into(),
            focused: Some(7),
            root: MigrateNode::Split {
                axis: "h".into(),
                ratio: 0.5,
                first: Box::new(MigrateNode::Pane {
                    pane: pane("local"),
                }),
                second: Box::new(MigrateNode::Split {
                    axis: "v".into(),
                    ratio: 0.25,
                    first: Box::new(MigrateNode::Pane {
                        pane: pane("remote:h1"),
                    }),
                    second: Box::new(MigrateNode::Pane {
                        pane: pane("local"),
                    }),
                }),
            },
        }
    }

    #[test]
    fn migrate_tab_roundtrip_nested_splits() {
        let tab = nested_tab();
        let json = serde_json::to_string(&tab).unwrap();
        assert!(json.contains(r#""title":"work""#), "{json}");
        assert!(
            json.contains(r#""axis":"v""#) && json.contains(r#""ratio":0.25"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<MigrateTab>(&json).unwrap(), tab);
        // focused is optional on the wire (unfocused tab)
        let bare: MigrateTab = serde_json::from_str(
            r#"{"title":"t","focused":null,"root":{"pane":{"pane":{"manual_title":null,"kind":"local","degraded":false,"pid":1,"cols":2,"rows":3}}}}"#,
        )
        .unwrap();
        assert_eq!(bare.focused, None);
        assert_eq!(
            bare.root,
            MigrateNode::Pane {
                pane: MigratePane {
                    manual_title: None,
                    kind: "local".into(),
                    degraded: false,
                    pid: 1,
                    cols: 2,
                    rows: 3,
                }
            }
        );
    }

    #[test]
    fn request_response_wire_shapes() {
        let out: Request = serde_json::from_str(
            r#"{"cmd":"migrate_out","pane":"work","target":"/run/user/1000/terminator-rust/ipc.sock"}"#,
        )
        .unwrap();
        assert_eq!(
            out,
            Request::MigrateOut {
                pane: PaneSelector::Name("work".into()),
                target: "/run/user/1000/terminator-rust/ipc.sock".into(),
            }
        );
        assert_eq!(
            serde_json::to_string(&out).unwrap(),
            r#"{"cmd":"migrate_out","pane":"work","target":"/run/user/1000/terminator-rust/ipc.sock"}"#
        );
        let by_id: Request =
            serde_json::from_str(r#"{"cmd":"migrate_out","pane":3,"target":"x"}"#).unwrap();
        assert_eq!(
            by_id,
            Request::MigrateOut {
                pane: PaneSelector::Id(3),
                target: "x".into(),
            }
        );

        let tab_json = serde_json::to_string(&nested_tab()).unwrap();
        let offer: Request =
            serde_json::from_str(&format!(r#"{{"cmd":"tab_offer","tab":{tab_json}}}"#)).unwrap();
        assert_eq!(offer, Request::TabOffer { tab: nested_tab() });
        let offer_out = serde_json::to_string(&offer).unwrap();
        assert!(
            offer_out.starts_with(r#"{"cmd":"tab_offer","tab":{"title":"work""#),
            "{offer_out}"
        );

        assert_eq!(
            serde_json::to_string(&Response::Migrated { panes: 3 }).unwrap(),
            r#"{"ok":"migrated","panes":3}"#
        );
        let back: Response = serde_json::from_str(r#"{"ok":"migrated","panes":3}"#).unwrap();
        assert_eq!(back, Response::Migrated { panes: 3 });
    }

    #[test]
    fn payload_roundtrip_and_layout() {
        assert_eq!(encode_payload(&[]), Vec::<u8>::new());
        assert_eq!(encode_payload(&[None]), vec![0, 0, 0, 0]);
        assert_eq!(
            encode_payload(&[Some(b"hi".to_vec())]),
            vec![2, 0, 0, 0, b'h', b'i']
        );
        let snaps = vec![
            Some(b"abc".to_vec()),
            None,
            Some(vec![0u8; 300]), // multi-byte length prefix (LE)
        ];
        let enc = encode_payload(&snaps);
        assert_eq!(decode_payload(&enc, 3).unwrap(), snaps);
        assert_eq!(decode_payload(&[0, 0, 0, 0], 1).unwrap(), vec![None]);
        assert_eq!(
            decode_payload(&[], 0).unwrap(),
            Vec::<Option<Vec<u8>>>::new()
        );
    }

    #[test]
    fn payload_truncated_or_trailing() {
        // declared length longer than the bytes present
        let err = decode_payload(&[3, 0, 0, 0, b'a'], 1).unwrap_err();
        assert!(err.contains("truncated"), "{err}");
        // missing length prefix entirely
        let err = decode_payload(&[], 1).unwrap_err();
        assert!(err.contains("truncated"), "{err}");
        // fewer blocks than n
        let err = decode_payload(&[1, 0, 0, 0, b'a', 5, 0, 0, 0, b'b'], 2).unwrap_err();
        assert!(err.contains("truncated"), "{err}");
        // payload holds more blocks than n claims -> trailing garbage
        let err = decode_payload(&[1, 0, 0, 0, b'a', 1, 0, 0, 0, b'b'], 1).unwrap_err();
        assert!(err.contains("trailing"), "{err}");
    }

    #[test]
    fn payload_max_enforced() {
        let big = vec![0u8; MAX_MIGRATE_PAYLOAD + 1];
        let err = decode_payload(&big, 0).unwrap_err();
        assert!(err.contains("exceeds limit"), "{err}");
        // exactly at the limit passes the size gate (fails on shape, not
        // size: no block claimed for the trailing bytes)
        let edge = vec![0u8; MAX_MIGRATE_PAYLOAD];
        let err = decode_payload(&edge, 0).unwrap_err();
        assert!(
            err.contains("trailing") && !err.contains("exceeds"),
            "{err}"
        );
    }

    #[test]
    fn discover_sockets_filters_skips_and_sorts() {
        let dir = std::env::temp_dir().join(format!(
            "ipc-proto-migrate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "ipc.sock",
            "ipc-123.sock",
            "ipcfoo.sock",
            "other.txt",
            "ip.sock",
            "ipc.soc",
        ] {
            std::fs::write(dir.join(name), b"").unwrap();
        }

        let all = discover_sockets(&dir, None);
        let names: Vec<String> = all
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["ipc-123.sock", "ipc.sock", "ipcfoo.sock"]);

        let skipped = discover_sockets(&dir, Some(&dir.join("ipc.sock")));
        let names: Vec<String> = skipped
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["ipc-123.sock", "ipcfoo.sock"]);

        // a skip path that does not exist filters nothing; a missing dir
        // yields an empty list rather than an error
        let none = discover_sockets(&dir, Some(&dir.join("nope.sock")));
        assert_eq!(none.len(), 3);
        assert!(discover_sockets(&dir.join("nope-dir"), None).is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn leaf_count_walks_depth_first() {
        assert_eq!(
            leaf_count(&MigrateNode::Pane {
                pane: pane("local")
            }),
            1
        );
        assert_eq!(leaf_count(&nested_tab().root), 3);
    }
}
