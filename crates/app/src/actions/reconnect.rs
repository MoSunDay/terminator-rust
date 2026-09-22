//! Automatic reconnection of remote panes whose ssh died with a
//! connection-failure exit (network drop): the zellij session survives
//! on the host, so the pane is KEPT (never corpse-closed) and reattached
//! through the idempotent bootstrap with a short backoff until the
//! network is back - recovery is then immediate.

use std::time::{Duration, Instant};

use layout_tree::PaneId;
use log::info;
use remote::{is_disconnect, PaneKind};

use crate::session_map::{self, SessionMap};
use crate::state::AppState;

/// First retry delay after a dropped connection.
const RECONNECT_BASE: Duration = Duration::from_secs(1);
/// Backoff cap: once the network is back, a pane recovers within this.
const RECONNECT_MAX: Duration = Duration::from_secs(5);
/// A session that lived at least this long before dying was healthy:
/// the backoff ladder restarts from the bottom.
const STABLE_AFTER: Duration = Duration::from_secs(30);

/// Retry delay after `fails` consecutive failed reconnect attempts
/// (1s, 2s, 4s, then capped at 5s).
pub fn backoff(fails: u32) -> Duration {
    (RECONNECT_BASE * 2u32.saturating_pow(fails.saturating_sub(1))).min(RECONNECT_MAX)
}

/// True while the pane sits in the reconnect backoff window (ssh died,
/// retry not fired yet) - drawn as a badge by the pane header.
pub fn pending(sess: &SessionMap, pane: PaneId) -> bool {
    sess.reconnect_at.contains_key(&pane)
}

/// Per-frame reconnect pump: schedule retries for remote panes whose
/// exit means "connection lost", and fire the due ones (drop the dead
/// session; `ensure_sessions` respawns it with a fresh ssh attach in the
/// same frame).
pub fn pump(st: &AppState, sess: &mut SessionMap) {
    let now = Instant::now();
    // Fresh disconnects -> schedule the retry (dead session stays in the
    // map so its last frame + a "reconnecting" badge remain visible).
    let fresh: Vec<PaneId> = sess
        .map
        .iter()
        .filter(|(id, s)| {
            s.exit.is_some_and(is_disconnect)
                && !sess.reconnect_at.contains_key(*id)
                && matches!(
                    st.panes.get(*id).map(|m| &m.kind),
                    Some(PaneKind::Remote(_))
                )
        })
        .map(|(id, _)| *id)
        .collect();
    for id in fresh {
        let prev = sess.reconnect_n.get(&id).copied().unwrap_or(0);
        let lived = sess
            .spawned_at
            .get(&id)
            .map_or(Duration::ZERO, |t| now.duration_since(*t));
        let fails = if lived >= STABLE_AFTER || prev == 0 {
            1
        } else {
            prev + 1
        };
        sess.reconnect_n.insert(id, fails);
        sess.reconnect_at.insert(id, now + backoff(fails));
        info!(
            "pane {id}: connection lost, retrying in {:?}",
            backoff(fails)
        );
    }
    // Due retries: dropping the dead session is enough, ensure_sessions
    // respawns it (fresh `zellij attach --create`, same session).
    let due: Vec<PaneId> = sess
        .reconnect_at
        .iter()
        .filter(|(_, at)| **at <= now)
        .map(|(id, _)| *id)
        .collect();
    for id in due {
        if st.panes.contains_key(&id) {
            info!(
                "pane {id}: reconnecting (attempt {})",
                sess.reconnect_n.get(&id).copied().unwrap_or(1)
            );
            session_map::terminate(sess, id);
        } else {
            sess.reconnect_at.remove(&id);
            sess.reconnect_n.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::close_exited;
    use crate::session_map::EXIT_GRACE;
    use crate::state::{fresh_state, ui_state};
    use remote::RemoteTarget;
    use vt_pane::{task as vtask, SessionOpts};

    /// A real short-lived session that has already exited with `status`.
    fn dead_session(status: i32) -> vt_pane::Session {
        let code = status.to_string();
        let opts = SessionOpts {
            cols: 10,
            rows: 5,
            argv: vec!["/bin/sh".into(), "-c".into(), format!("exit {code}")],
            env: Vec::new(),
            scrollback_lines: 100,
            dark: true,
        };
        let mut s = vtask::spawn_session(&opts).expect("spawn sh");
        for _ in 0..250 {
            let _ = vtask::pump(&mut s);
            if s.exit.is_some() {
                assert_eq!(s.exit, Some(status));
                return s;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("test session never exited");
    }

    /// Fresh single-pane state whose pane 1 is a remote of `target`.
    fn remote_state() -> AppState {
        let mut st = fresh_state();
        if let Some(m) = st.panes.get_mut(&1) {
            m.kind = PaneKind::Remote(RemoteTarget {
                label: "t".to_string(),
                host: "nowhere.invalid".to_string(),
                user: None,
                port: None,
                session_name: "work".to_string(),
            });
        }
        st
    }

    /// Mark pane 1's session as exited past the grace window.
    fn ripen(sess: &mut SessionMap, status: i32) {
        sess.map.insert(1, dead_session(status));
        sess.exited_seen
            .insert(1, Instant::now() - EXIT_GRACE - Duration::from_millis(50));
    }

    #[test]
    fn backoff_ladder() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(3), Duration::from_secs(4));
        assert_eq!(backoff(4), Duration::from_secs(5));
        assert_eq!(backoff(9), Duration::from_secs(5));
    }

    #[test]
    fn remote_drop_is_scheduled_and_survives_close() {
        let mut st = remote_state();
        let mut sess = session_map::session_map();
        let mut ui = ui_state();
        let mut dirty = false;
        ripen(&mut sess, 255);
        pump(&st, &mut sess);
        assert!(pending(&sess, 1), "remote drop schedules a reconnect");
        assert_eq!(sess.reconnect_n.get(&1), Some(&1));
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        assert!(
            st.panes.contains_key(&1),
            "disconnect pane is kept for reattachment"
        );
    }

    #[test]
    fn local_exit_is_not_reconnected() {
        let mut st = fresh_state();
        let mut sess = session_map::session_map();
        let mut ui = ui_state();
        let mut dirty = false;
        ripen(&mut sess, 255);
        pump(&st, &mut sess);
        assert!(!pending(&sess, 1), "local panes never reconnect");
        close_exited(&mut st, &mut sess, &mut ui, &mut dirty);
        assert!(!st.panes.contains_key(&1), "local corpse still closes");
    }

    #[test]
    fn exit42_is_not_a_disconnect() {
        let st = remote_state();
        let mut sess = session_map::session_map();
        ripen(&mut sess, 42);
        pump(&st, &mut sess);
        assert!(!pending(&sess, 1), "auto_degrade owns exit 42");
    }

    #[test]
    fn due_retry_drops_the_dead_session() {
        let st = remote_state();
        let mut sess = session_map::session_map();
        ripen(&mut sess, 255);
        pump(&st, &mut sess);
        sess.reconnect_at
            .insert(1, Instant::now() - Duration::from_secs(1));
        pump(&st, &mut sess);
        assert!(
            !sess.map.contains_key(&1) && !sess.reconnect_at.contains_key(&1),
            "due retry dropped the dead session for ensure_sessions"
        );
    }

    #[test]
    fn failed_retry_grows_the_ladder() {
        let st = remote_state();
        let mut sess = session_map::session_map();
        ripen(&mut sess, 255);
        pump(&st, &mut sess);
        assert_eq!(sess.reconnect_n.get(&1), Some(&1));
        // Retry fires, ensure_sessions respawns, the fresh ssh dies at
        // once again (network still down): the delay must GROW.
        sess.reconnect_at
            .insert(1, Instant::now() - Duration::from_secs(1));
        pump(&st, &mut sess);
        session_map::note_spawned(&mut sess, 1, dead_session(255));
        ripen(&mut sess, 255);
        pump(&st, &mut sess);
        assert_eq!(
            sess.reconnect_n.get(&1),
            Some(&2),
            "quick consecutive failures grow the backoff"
        );
    }

    #[test]
    fn stable_run_resets_the_ladder() {
        let st = remote_state();
        let mut sess = session_map::session_map();
        ripen(&mut sess, 255);
        sess.spawned_at
            .insert(1, Instant::now() - Duration::from_secs(40));
        sess.reconnect_n.insert(1, 3);
        pump(&st, &mut sess);
        assert_eq!(
            sess.reconnect_n.get(&1),
            Some(&1),
            "a healthy run restarts the ladder"
        );
    }
}
