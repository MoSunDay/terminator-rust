//! pane-id -> Session management plus thin vt_pane wrappers.
//!
//! Spawning maps `PaneMeta` kinds to remote spawn plans (local shell,
//! ssh+zellij bootstrap, degraded plain ssh) and owns the live `Session`
//! objects. No egui types here.

use std::collections::HashMap;

use anyhow::Result;
use layout_tree::PaneId;
use log::warn;
use remote::{local_plan, remote_plan, PaneKind, SpawnPlan};
use vt_pane::{task as vtask, Frame as VtFrame, Session, SessionOpts};

use crate::render::colors;
use crate::state::{compute_grid, CellSize, PaneMeta};

/// Terminal size used at spawn time; resized to the real pane on first frame.
pub const START_COLS: u16 = 80;
pub const START_ROWS: u16 = 24;

pub struct SessionMap {
    pub map: HashMap<PaneId, Session>,
}

pub fn session_map() -> SessionMap {
    SessionMap {
        map: HashMap::new(),
    }
}

fn opts(plan: &SpawnPlan, cols: u16, rows: u16) -> SessionOpts {
    SessionOpts {
        cols,
        rows,
        argv: plan.argv.clone(),
        env: plan.env.clone(),
        scrollback_lines: 10_000,
    }
}

/// Spawn a session for a pane per its kind.
///
/// Local panes run the login shell. Remote panes run the zellij bootstrap
/// with the current theme palette; degraded panes (host without zellij or
/// after a degraded respawn) get a plain `ssh -tt` shell.
pub fn spawn_meta(meta: &PaneMeta, theme_name: &str, cols: u16, rows: u16) -> Result<Session> {
    match &meta.kind {
        PaneKind::Local => vtask::spawn_session(&opts(&local_plan(), cols, rows)),
        PaneKind::Remote(target) => {
            if meta.degraded {
                let plan = remote_plan(target, remote::DEFAULT_PALETTE_HEX, false);
                vtask::spawn_session(&opts(&plan, cols, rows))
            } else {
                let owned = colors::palette_hex(&colors::palette_of(theme_name));
                let hex: [&str; 9] = std::array::from_fn(|i| owned[i].as_str());
                let plan = remote_plan(target, hex, true);
                vtask::spawn_session(&opts(&plan, cols, rows))
            }
        }
    }
}

/// Drain pty bytes for every live session (call each frame).
pub fn pump_all(sess: &mut SessionMap) {
    for (id, s) in sess.map.iter_mut() {
        if let Err(e) = vtask::pump(s) {
            warn!("pump pane {id}: {e}");
        }
    }
}

/// Pump one session, resize the terminal to the content area if needed and
/// return the render snapshot.
pub fn sync_frame(sess: &mut Session, w: f32, h: f32, cell: CellSize) -> VtFrame {
    if let Err(e) = vtask::pump(sess) {
        warn!("pump: {e}");
    }
    let (cols, rows) = compute_grid(w, h, cell.w, cell.h);
    let cur = match vtask::frame(sess) {
        Ok(f) => f,
        Err(e) => {
            warn!("frame: {e}");
            return VtFrame::default();
        }
    };
    if cur.cols != cols || cur.rows != rows {
        if let Err(e) = vtask::resize(sess, cols, rows, cell.w_px, cell.h_px) {
            warn!("resize: {e}");
        }
        return vtask::frame(sess).unwrap_or_default();
    }
    cur
}

/// OSC 0/2 title of a pane's session ("" when absent).
pub fn osc_title(sess: &SessionMap, pane: PaneId) -> String {
    sess.map.get(&pane).map(vtask::title).unwrap_or_default()
}

/// Observed exit code of a pane's child, if any.
pub fn exit_code(sess: &SessionMap, pane: PaneId) -> Option<i32> {
    sess.map.get(&pane).and_then(|s| s.exit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;
    use vt_pane::SessionOpts;

    fn drain_until_text(sess: &mut Session, needle: &str) -> VtFrame {
        for _ in 0..200 {
            let _ = vtask::pump(sess);
            if let Ok(f) = vtask::frame(sess) {
                let text: String = f.cells.iter().flatten().map(|c| c.text.as_str()).collect();
                if text.contains(needle) {
                    return f;
                }
            }
            sleep(Duration::from_millis(10));
        }
        VtFrame::default()
    }

    #[test]
    fn command_session_pumps_output_into_frame() {
        let opts = SessionOpts::command(20, 5, vec!["printf".into(), "hello-app".into()]);
        let mut sess = match vtask::spawn_session(&opts) {
            Ok(s) => s,
            Err(e) => {
                // CI without a usable pty: skip rather than fail.
                eprintln!("skip: {e}");
                return;
            }
        };
        let f = drain_until_text(&mut sess, "hello-app");
        let text: String = f.cells.iter().flatten().map(|c| c.text.as_str()).collect();
        assert!(text.contains("hello-app"), "frame text: {text:?}");
    }

    #[test]
    fn sync_frame_resizes_to_requested_grid() {
        let opts = SessionOpts::command(10, 3, vec!["true".into()]);
        let mut sess = match vtask::spawn_session(&opts) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let cell = CellSize { w: 8.0, h: 16.0, w_px: 8, h_px: 16 };
        let f = sync_frame(&mut sess, 160.0, 80.0, cell);
        assert_eq!((f.cols, f.rows), (20, 5));
    }

    #[test]
    fn local_meta_spawns_shell() {
        let meta = PaneMeta {
            kind: PaneKind::Local,
            manual_title: None,
            bg_color: None,
            transparency: 0.0,
            degraded: false,
        };
        assert!(spawn_meta(&meta, "dracula", 10, 5).is_ok());
    }
}
