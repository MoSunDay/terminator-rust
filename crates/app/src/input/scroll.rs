//! Local scrollback review keys (desktop-terminal habit): Shift+PageUp /
//! Shift+PageDown page through history, Shift+Home / Shift+End jump to the
//! top / back to live.

use egui::{Key, Modifiers};
use vt_pane::{viewport, Session};

/// Handle one key as a local scrollback action; true when consumed.
///
/// Only bare-Shift combos qualify: any ctrl/alt/command involvement keeps
/// the key on the normal path (app shortcut or child input), matching the
/// GNOME Terminal habit baseline.
pub fn handle_key(sess: &mut Session, key: Key, mods: &Modifiers) -> bool {
    if !mods.shift || mods.ctrl || mods.alt || mods.command {
        return false;
    }
    let rows = sess.term.rows().unwrap_or(24);
    match key {
        Key::PageUp => {
            viewport::scroll_by(sess, viewport::page_delta(rows, true));
            true
        }
        Key::PageDown => {
            viewport::scroll_by(sess, viewport::page_delta(rows, false));
            true
        }
        Key::Home => {
            viewport::scroll_top(sess);
            true
        }
        Key::End => {
            viewport::scroll_bottom(sess);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vt_pane::{task as vtask, SessionOpts};

    /// `sh -c 'seq 1 300; cat'` on a real 20x5 pty: seq guarantees deep
    /// scrollback, cat keeps the session alive and idle. Skips gracefully
    /// when no pty is available.
    fn harness() -> Option<Session> {
        let opts = SessionOpts::command(
            20,
            5,
            vec![
                "sh".to_string(),
                "-c".to_string(),
                "seq 1 300; cat".to_string(),
            ],
        );
        match vtask::spawn_session(&opts) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("skip: {e}");
                None
            }
        }
    }

    fn grid_text(s: &mut Session) -> String {
        let _ = vtask::pump(s);
        match vtask::frame(s) {
            Ok(f) => f.cells.iter().flatten().map(|c| c.text.as_str()).collect(),
            Err(_) => String::new(),
        }
    }

    /// Pump until the grid shows the needle (seq flushed, cat waiting).
    fn wait_grid(s: &mut Session, needle: &str) -> bool {
        for _ in 0..100 {
            if grid_text(s).contains(needle) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    fn shift_only() -> Modifiers {
        Modifiers {
            shift: true,
            ..Default::default()
        }
    }

    #[test]
    fn page_up_reviews_and_end_returns_live() {
        let Some(mut s) = harness() else { return };
        assert!(
            wait_grid(&mut s, "300"),
            "seq output never reached the grid"
        );
        assert!(viewport::pinned(&s), "fresh session should start pinned");
        let (total_before, len) = match viewport::geometry(&s) {
            Some((_, total, len)) => (total, len),
            None => return,
        };
        assert!(total_before > len, "expected history before the viewport");

        assert!(handle_key(&mut s, Key::PageUp, &shift_only()));
        assert!(!viewport::pinned(&s), "PageUp should scroll into history");
        // One page up from live: offset = total - len - (rows - 1).
        let expect = total_before - len - 4;
        assert_eq!(viewport::geometry(&s).map(|g| g.0), Some(expect));

        assert!(handle_key(&mut s, Key::End, &shift_only()));
        assert!(viewport::pinned(&s), "End should return to the live area");
        assert!(viewport::geometry(&s).is_some_and(|g| g.0 + g.2 == g.1));
    }

    #[test]
    fn home_lands_at_the_top_of_history() {
        let Some(mut s) = harness() else { return };
        assert!(
            wait_grid(&mut s, "300"),
            "seq output never reached the grid"
        );
        assert!(handle_key(&mut s, Key::Home, &shift_only()));
        assert!(!viewport::pinned(&s), "Home should scroll into history");
        assert_eq!(viewport::geometry(&s).map(|g| g.0), Some(0));
        assert!(
            !grid_text(&mut s).contains("300"),
            "top of history must show line 1"
        );
    }

    #[test]
    fn command_combos_and_plain_keys_are_not_consumed() {
        let Some(mut s) = harness() else { return };
        let shift = shift_only();
        let ctrl_shift = Modifiers {
            ctrl: true,
            ..shift
        };
        let alt_shift = Modifiers { alt: true, ..shift };
        // App-shortcut territory (route_shortcut runs first anyway) and
        // unmodified keys must fall through to the child untouched.
        assert!(!handle_key(&mut s, Key::PageUp, &ctrl_shift));
        assert!(!handle_key(&mut s, Key::PageUp, &alt_shift));
        assert!(!handle_key(&mut s, Key::PageUp, &Modifiers::default()));
        assert!(!handle_key(&mut s, Key::A, &shift));
        assert!(viewport::pinned(&s), "viewport must not have moved");
    }
}
