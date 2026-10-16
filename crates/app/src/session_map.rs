//! pane-id -> Session management plus thin vt_pane wrappers.
//!
//! Spawning maps `PaneMeta` kinds to remote spawn plans (local shell,
//! ssh+remote keeper bootstrap, degraded plain ssh) and owns live `Session`
//! objects. No egui types here.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use layout_tree::PaneId;
use log::warn;
use remote::{remote_plan, PaneKind, SpawnPlan};
use vt_pane::{task as vtask, Frame as VtFrame, Session, SessionOpts, Wake};

use crate::render::colors;
use crate::state::{compute_grid, CellSize, PaneMeta};
use theme::{Palette, Rgb};

/// Terminal size used at spawn time; resized to the real pane on first frame.
pub const START_COLS: u16 = 80;
pub const START_ROWS: u16 = 24;

/// Change-detection key of the last [`crate::terminal_theme::sync`] run
/// (stored in [`SessionMap::theme_sync`]). Equal key = every session that
/// exists NOW already carries exactly this palette + effective background,
/// so the sync can return without touching a single terminal.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeSyncKey {
    /// The resolved palette as a VALUE (no `Eq`: Palette derives only
    /// PartialEq). A name-keyed memo missed `bg_color`/transparency edits
    /// that keep the theme name while moving the resolved table, so the
    /// sync returned early and stale render snapshots were replayed.
    pub palette: Palette,
    /// Effective background override the palette was applied with
    /// (settings `bg_color`, `None` = the palette's own background).
    pub bg: Option<Rgb>,
    /// [`SessionMap::session_gen`] when that run happened.
    pub gen: u64,
}

pub struct SessionMap {
    pub map: HashMap<PaneId, Session>,
    /// Reader-thread wakeup injected into every spawned/adopted session
    /// (egui `request_repaint`): pty output wakes the render loop, so no
    /// polling tick is needed. Set once at app startup.
    pub wake: Option<Wake>,
    /// Earliest allowed re-spawn per pane after a spawn failure (backoff).
    pub retry_at: HashMap<PaneId, Instant>,
    /// First moment each pane's EXITED session was seen; auto-close waits
    /// [`EXIT_GRACE`] so a shell that dies instantly cannot fork-loop the
    /// empty-tree respawn (and its last output stays briefly readable).
    pub exited_seen: HashMap<PaneId, Instant>,
    /// Earliest allowed reconnect attempt per remote pane whose ssh died
    /// with a connection-failure exit.
    pub reconnect_at: HashMap<PaneId, Instant>,
    /// Consecutive failed reconnect attempts (backoff step).
    pub reconnect_n: HashMap<PaneId, u32>,
    /// When the pane's current session was spawned (a healthy run resets
    /// the backoff ladder).
    pub spawned_at: HashMap<PaneId, Instant>,
    /// Bumped whenever a session ENTERS or LEAVES `map` (the three
    /// production mutation points are `note_spawned`, `terminate` and
    /// `detach`; tests that insert straight into `map` bypass it, which
    /// only costs them a re-apply). Every other `map` access mutates a
    /// session in place and cannot change which sessions need a theme.
    pub session_gen: u64,
    /// Key of the last [`crate::terminal_theme::sync`] run (palette + bg +
    /// session generation), or `None` before the first one. The sync runs
    /// twice per frame - once before the window passes, once after a pass
    /// may have spawned sessions - and rebuilding a 256-slot palette per
    /// session per call is pure overhead once nothing moved.
    pub theme_sync: Option<ThemeSyncKey>,
}

/// Wait after a failed spawn before retrying (stops per-frame fork storms).
pub const SPAWN_BACKOFF: Duration = Duration::from_secs(2);

/// How long an exited session lingers before `actions::close_exited`
/// removes its pane.
pub const EXIT_GRACE: Duration = Duration::from_millis(250);

pub fn session_map() -> SessionMap {
    SessionMap {
        map: HashMap::new(),
        wake: None,
        retry_at: HashMap::new(),
        exited_seen: HashMap::new(),
        reconnect_at: HashMap::new(),
        reconnect_n: HashMap::new(),
        spawned_at: HashMap::new(),
        session_gen: 0,
        theme_sync: None,
    }
}

/// Note that a session entered or left [`SessionMap::map`]: the theme sync
/// key must not survive it (the new session still needs its palette).
fn bump_session_gen(sess: &mut SessionMap) {
    sess.session_gen = sess.session_gen.wrapping_add(1);
}

/// True while the pane's post-failure backoff window is still running.
pub fn spawn_blocked(sess: &SessionMap, id: PaneId, now: Instant) -> bool {
    sess.retry_at.get(&id).is_some_and(|at| *at > now)
}

/// Terminate a pane's child process group, stop its reader thread and
/// drop the session. The single removal path for live sessions.
/// `reconnect_n` deliberately survives: it counts consecutive failed
/// attempts across respawns and is reset only by a stable run or a
/// manual respawn (pane ids are never reused, so stale entries are inert).
pub fn terminate(sess: &mut SessionMap, id: PaneId) {
    if let Some(mut s) = sess.map.remove(&id) {
        vtask::terminate(&mut s);
    }
    sess.retry_at.remove(&id);
    sess.exited_seen.remove(&id);
    sess.reconnect_at.remove(&id);
    bump_session_gen(sess);
}

/// How long `detach` waits for the reader thread to close the master fd.
pub const DETACH_WAIT: Duration = Duration::from_millis(400);

/// Drop a session WITHOUT signalling its child. The one removal path for
/// panes whose PTY master has been handed to another instance (migration):
/// the child must keep running there. The reader thread is stopped and
/// given a moment to close the master fd, which is why callers must have
/// taken their own `dup` of it first (see `vtask::dup_master`).
pub fn detach(sess: &mut SessionMap, id: PaneId) {
    if let Some(s) = sess.map.remove(&id) {
        vtask::stop_reader(&s);
        let deadline = Instant::now() + DETACH_WAIT;
        while !vtask::reader_done(&s) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        drop(s);
    }
    sess.retry_at.remove(&id);
    sess.exited_seen.remove(&id);
    sess.reconnect_at.remove(&id);
    bump_session_gen(sess);
}

/// Options for ADOPTING an existing session rather than spawning one:
/// `argv`/`env` are unused past the fork, so only the terminal-facing
/// policy (size, scrollback, color-scheme answer) is carried over.
pub fn adopt_opts(cols: u16, rows: u16, dark: bool, wake: &Option<Wake>) -> SessionOpts {
    SessionOpts {
        cols,
        rows,
        argv: Vec::new(),
        env: Vec::new(),
        scrollback_lines: 10_000,
        dark,
        wake: wake.clone(),
    }
}

/// Insert a freshly spawned session and clear the respawn bookkeeping
/// markers that must not outlive a spawn (backoff, exit timestamp,
/// pending reconnect plan), stamping the spawn time for the reconnect
/// backoff reset. `reconnect_n` survives so quick consecutive failures
/// keep growing the retry delay.
pub fn note_spawned(sess: &mut SessionMap, id: PaneId, s: Session) {
    sess.map.insert(id, s);
    sess.retry_at.remove(&id);
    sess.exited_seen.remove(&id);
    sess.reconnect_at.remove(&id);
    sess.spawned_at.insert(id, Instant::now());
    bump_session_gen(sess);
}

fn local_plan() -> SpawnPlan {
    let options = SessionOpts::local_shell(80, 24);
    SpawnPlan {
        argv: options.argv,
        env: options.env,
    }
}

fn opts(
    plan: &SpawnPlan,
    cols: u16,
    rows: u16,
    dark: bool,
    pane: PaneId,
    wake: &Option<Wake>,
) -> SessionOpts {
    let mut env = plan.env.clone();
    env.push(format!("{}={pane}", ipc_proto::ENV_PANE_ID));
    if let Ok(path) = std::env::current_exe() {
        let ctl = path.with_file_name("terminator-ctl");
        if ctl.is_file() {
            env.push(format!("{}={}", ipc_proto::ENV_CTL, ctl.display()));
        }
    }
    SessionOpts {
        cols,
        rows,
        argv: plan.argv.clone(),
        env,
        scrollback_lines: 10_000,
        dark,
        wake: wake.clone(),
    }
}

/// Spawn a session for a pane per its kind.
///
/// Local panes run the login shell. Remote panes use terminator-session;
/// degraded panes (host without it) get a plain `ssh -tt` shell.
pub fn spawn_meta(
    meta: &PaneMeta,
    theme_name: &str,
    cols: u16,
    rows: u16,
    pane: PaneId,
    wake: &Option<Wake>,
) -> Result<Session> {
    let dark = colors::is_dark(theme_name);
    match &meta.kind {
        PaneKind::Local => vtask::spawn_session(&opts(&local_plan(), cols, rows, dark, pane, wake)),
        PaneKind::Remote(target) => {
            if meta.degraded {
                let plan = remote_plan(target, false);
                vtask::spawn_session(&opts(&plan, cols, rows, dark, pane, wake))
            } else {
                let plan = remote_plan(target, true);
                vtask::spawn_session(&opts(&plan, cols, rows, dark, pane, wake))
            }
        }
    }
}

/// Drain pty bytes for every live session (call each frame). Returns the
/// panes whose terminal reported a desktop notification (OSC 9/777) since
/// the previous pump, for the attention badge.
pub fn pump_all(sess: &mut SessionMap) -> Vec<PaneId> {
    let mut noticed = Vec::new();
    for (id, s) in sess.map.iter_mut() {
        if let Err(e) = vtask::pump(s) {
            warn!("pump pane {id}: {e}");
        }
        if vtask::take_notice(s) {
            noticed.push(*id);
        }
    }
    noticed
}

/// One pane's per-frame render input: the render snapshot plus the
/// frame-cache generation it was built at (see [`sync_frame`]).
pub struct FrameSync<'a> {
    /// [`vtask::frame_gen`] covering `frame`. Render-side shape caches key
    /// on it, so it must be read in the SAME call that hands out the
    /// snapshot - a key read before a later resize/invalidation would be
    /// one generation behind the painted content.
    pub gen: u64,
    /// Terminal snapshot, cached between changes (`vtask::frame_cached`).
    pub frame: &'a VtFrame,
}

/// Resize the terminal to the content area if needed and return the render
/// snapshot (cached between changes - see `vtask::frame_cached` - so idle
/// frames cost no snapshot churn) together with the generation covering it.
///
/// Does NOT pump: [`pump_all`] drains every session's pty ONCE per frame
/// before any render pass (main.rs), and the ipc drain - which also pumps a
/// captured pane - runs there too, so pumping here re-drained nothing while
/// invalidating the snapshot mid-frame: every output burst then tessellated
/// twice (the pass painted the fresh snapshot under a stale generation, the
/// next pass rebuilt the same shapes under the new one). A session spawned
/// after this frame's pump gets its first bytes in the next frame's
/// `pump_all`, which its reader wake already schedules.
pub fn sync_frame(sess: &mut Session, w: f32, h: f32, cell: CellSize) -> FrameSync<'_> {
    let (cols, rows) = compute_grid(w, h, cell.w, cell.h);
    let stale_grid = {
        let cur = vtask::frame_cached(sess);
        cur.cols != cols || cur.rows != rows
    };
    if stale_grid {
        if let Err(e) = vtask::resize(sess, cols, rows, cell.w_px, cell.h_px) {
            warn!("resize: {e}");
        }
        vtask::invalidate_frame(sess);
    }
    // Read the generation AFTER the resize above bumped it (a rebuild by
    // `frame_cached` does not move it), so key and snapshot agree.
    let gen = vtask::frame_gen(sess);
    FrameSync {
        gen,
        frame: vtask::frame_cached(sess),
    }
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
    fn pump_all_reports_a_pane_that_emitted_the_notice_osc_once() {
        // printf renders the exact bytes the remote keeper injects
        // (\007 = BEL); the reader thread loops them back as PTY output.
        let opts = SessionOpts::command(
            20,
            5,
            vec![
                "printf".into(),
                "\\033]9;terminator-rust notice\\007".into(),
            ],
        );
        let sess = match vtask::spawn_session(&opts) {
            Ok(s) => s,
            Err(e) => {
                // CI without a usable pty: skip rather than fail.
                eprintln!("skip: {e}");
                return;
            }
        };
        let mut map = session_map();
        note_spawned(&mut map, 7, sess);
        let mut seen = false;
        for _ in 0..200 {
            if pump_all(&mut map).contains(&7) {
                seen = true;
                break;
            }
            sleep(Duration::from_millis(10));
        }
        assert!(seen, "OSC 9 pane never surfaced via pump_all");
        // The mark is consumed by the take: no further notification
        // means the pane is never reported again.
        assert!(!pump_all(&mut map).contains(&7));
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
        let cell = CellSize {
            w: 8.0,
            h: 16.0,
            wide_size: 0.0,
            w_px: 8,
            h_px: 16,
        };
        let f = sync_frame(&mut sess, 160.0, 80.0, cell);
        assert_eq!((f.frame.cols, f.frame.rows), (20, 5));
        let f_gen = f.gen;
        // The returned generation covers the snapshot: a resize just
        // invalidated it, and the next call (nothing moved) reports the
        // same generation with the same grid.
        let again = sync_frame(&mut sess, 160.0, 80.0, cell);
        assert_eq!((again.frame.cols, again.frame.rows), (20, 5));
        assert_eq!(again.gen, f_gen, "unchanged pane must not re-bump the gen");
    }

    #[test]
    fn local_meta_spawns_shell() {
        let meta = PaneMeta {
            kind: PaneKind::Local,
            manual_title: None,
            degraded: false,
        };
        assert!(spawn_meta(&meta, "dracula", 10, 5, 1, &None).is_ok());
    }

    #[test]
    fn local_pane_exports_its_notice_identity() {
        let opts = opts(&local_plan(), 80, 24, true, 42, &None);
        let shell = SessionOpts::local_shell(80, 24);
        assert_eq!(opts.argv, shell.argv);
        assert!(opts.env.contains(&format!("SHELL={}", opts.argv[0])));
        assert!(opts
            .env
            .iter()
            .any(|entry| entry == "TERMINATOR_PANE_ID=42"));
    }
}
