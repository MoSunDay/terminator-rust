//! Keep VT query answers and indexed colors aligned with the painted theme.
use anyhow::Result;
use libghostty_vt::{
    style::{Palette as VtPalette, RgbColor},
    Terminal,
};
use theme::{Palette, Rgb};

use crate::session_map::{SessionMap, ThemeSyncKey};

fn rgb(c: Rgb) -> RgbColor {
    RgbColor {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

/// Apply the palette; `Ok(true)` when anything changed (the caller drops
/// cached render snapshots, whose default/cursor colors just moved).
pub fn apply(
    term: &mut Terminal<'static, 'static>,
    palette: &Palette,
    bg: Option<Rgb>,
) -> Result<bool> {
    let fg = rgb(palette.foreground);
    let bg = rgb(bg.unwrap_or(palette.background));
    let cursor = rgb(palette.cursor);
    let colors = VtPalette(std::array::from_fn(|i| {
        rgb(theme::indexed_color(palette, i as u8))
    }));
    let mut changed = false;
    // Defaults preserve OSC overrides chosen by applications.
    if term.default_fg_color()? != Some(fg) {
        term.set_default_fg_color(Some(fg))?;
        changed = true;
    }
    if term.default_bg_color()? != Some(bg) {
        term.set_default_bg_color(Some(bg))?;
        changed = true;
    }
    if term.default_cursor_color()? != Some(cursor) {
        term.set_default_cursor_color(Some(cursor))?;
        changed = true;
    }
    if term.default_color_palette()?.0 != colors.0 {
        term.set_default_color_palette(Some(colors))?;
        changed = true;
    }
    Ok(changed)
}

pub fn sync(sessions: &mut SessionMap, palette: &Palette, bg: Option<Rgb>) {
    // O(1) fast path: the previous call already applied exactly this
    // palette + background to every session that exists NOW (the session
    // generation covers spawns/adoptions/removals), so there is nothing a
    // re-apply could change. Both per-frame calls used to rebuild a
    // 256-slot palette per session even when nothing moved.
    if key_matches(sessions, palette, bg) {
        return;
    }
    // A terminal that rejected the palette keeps the stale key, so the
    // next call retries it instead of banking the failure.
    let mut failed = false;
    for session in sessions.map.values_mut() {
        match apply(&mut session.term, palette, bg) {
            Ok(true) => vt_pane::task::invalidate_frame(session),
            // `apply` says "nothing moved" for this terminal, but the
            // CACHE may predate a palette-value move (e.g. a session that
            // was adopted already carrying the new table): invalidate the
            // snapshot whenever the banked value itself moved.
            Ok(false)
                if sessions
                    .theme_sync
                    .as_ref()
                    .is_some_and(|key| render_palette_moved(key, palette, bg)) =>
            {
                vt_pane::task::invalidate_frame(session)
            }
            Ok(false) => {}
            Err(error) => {
                failed = true;
                log::warn!("terminal theme: {error}");
            }
        }
    }
    if !failed {
        let gen = sessions.session_gen;
        sessions.theme_sync = Some(ThemeSyncKey {
            palette: palette.clone(),
            bg,
            gen,
        });
    }
}

/// True when the last [`sync`] already covered this palette VALUE, background
/// and session generation: no session can be missing the current theme.
fn key_matches(sessions: &SessionMap, palette: &Palette, bg: Option<Rgb>) -> bool {
    sessions.theme_sync.as_ref().is_some_and(|key| {
        key.gen == sessions.session_gen && key.bg == bg && key.palette == *palette
    })
}

/// True when the move from the banked key to `palette`/`bg` touched
/// something a cached render snapshot already baked in (its default/cursor
/// colors, or any cell color resolved through the engine's 256-table).
/// A generation-only difference (a spawn/adopt) stays false: the existing
/// caches are still correct and do not need a rebuild.
fn render_palette_moved(prev: &ThemeSyncKey, palette: &Palette, bg: Option<Rgb>) -> bool {
    prev.bg != bg || prev.palette != *palette
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn theme_answers_osc_queries_and_preserves_application_overrides() {
        let mut term = Terminal::new(80, 24).expect("terminal");
        let output = Arc::new(Mutex::new(Vec::new()));
        let sink = output.clone();
        term.on_pty_write(move |_, bytes| sink.lock().unwrap().extend_from_slice(bytes))
            .unwrap();
        let palette = theme::builtin::dracula();
        let bg = Rgb {
            r: 16,
            g: 32,
            b: 48,
        };
        assert!(
            apply(&mut term, &palette, Some(bg)).unwrap(),
            "initial apply"
        );
        assert!(!apply(&mut term, &palette, Some(bg)).unwrap(), "idempotent");
        term.vt_write(b"\x1b]10;?\x07\x1b]11;?\x07\x1b]4;1;?\x07");
        let response = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(response.contains("10;rgb:f8f8/f8f8/f2f2"), "{response:?}");
        assert!(response.contains("11;rgb:1010/2020/3030"), "{response:?}");
        assert!(response.contains("4;1;rgb:ffff/5555/5555"), "{response:?}");
        term.vt_write(b"\x1b]11;#123456\x07\x1b]4;1;#abcdef\x07");
        let next = theme::builtin::kanagawa_wave();
        assert!(apply(&mut term, &next, None).unwrap(), "palette moved");
        assert_eq!(term.default_bg_color().unwrap(), Some(rgb(next.background)));
        assert_eq!(
            term.bg_color().unwrap(),
            Some(RgbColor {
                r: 18,
                g: 52,
                b: 86
            })
        );
        assert_eq!(
            term.color_palette().unwrap().0[1],
            RgbColor {
                r: 171,
                g: 205,
                b: 239
            }
        );
    }

    #[test]
    fn sync_key_only_matches_the_palette_bg_and_session_generation_it_banked() {
        let mut sessions = crate::session_map::session_map();
        let palette = theme::builtin::dracula();
        let bg = Some(Rgb { r: 1, g: 2, b: 3 });
        assert!(!key_matches(&sessions, &palette, bg), "nothing synced yet");
        sync(&mut sessions, &palette, bg);
        assert!(key_matches(&sessions, &palette, bg), "the sync banked it");
        // A different palette, a different effective background, or a
        // session entering/leaving the map re-arms the sync.
        assert!(!key_matches(
            &sessions,
            &theme::builtin::kanagawa_wave(),
            bg
        ));
        assert!(!key_matches(&sessions, &palette, None));
        assert!(!key_matches(
            &sessions,
            &palette,
            Some(Rgb { r: 1, g: 2, b: 4 })
        ));
        sessions.session_gen += 1;
        assert!(!key_matches(&sessions, &palette, bg), "generation moved");
        // Re-syncing under the new generation banks it again.
        sync(&mut sessions, &palette, bg);
        assert!(key_matches(&sessions, &palette, bg));
    }

    /// A short-lived real session (skips on a host without a usable pty).
    fn test_session() -> Option<vt_pane::Session> {
        let opts = vt_pane::SessionOpts::command(10, 3, vec!["true".into()]);
        match vt_pane::task::spawn_session(&opts) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("skip: {e}");
                None
            }
        }
    }

    #[test]
    fn sync_themes_a_session_spawned_after_the_first_sync() {
        let mut sessions = crate::session_map::session_map();
        let palette = theme::builtin::dracula();
        let fg = rgb(palette.foreground);
        let Some(first) = test_session() else { return };
        crate::session_map::note_spawned(&mut sessions, 1, first);
        sync(&mut sessions, &palette, None);
        assert!(key_matches(&sessions, &palette, None), "first sync banked");
        assert_eq!(
            sessions
                .map
                .get(&1)
                .expect("session")
                .term
                .default_fg_color()
                .unwrap(),
            Some(fg)
        );
        // A session spawned after that sync must still get the palette:
        // the generation in the key re-arms it (an over-eager memo would
        // leave the new terminal on the engine defaults).
        let Some(second) = test_session() else { return };
        crate::session_map::note_spawned(&mut sessions, 2, second);
        assert!(!key_matches(&sessions, &palette, None), "spawn re-arms");
        sync(&mut sessions, &palette, None);
        assert_eq!(
            sessions
                .map
                .get(&2)
                .expect("session")
                .term
                .default_fg_color()
                .unwrap(),
            Some(fg)
        );
    }

    /// The change key must compare palette VALUES, not names: a
    /// `bg_color`/transparency edit keeps the theme name while moving the
    /// resolved table, and the old name-keyed memo silently replayed stale
    /// render snapshots (default/cursor/cell colors built under the
    /// previous table).
    #[test]
    fn palette_value_move_rearms_sync_and_invalidates_frames() {
        let mut sessions = crate::session_map::session_map();
        let palette = theme::builtin::dracula();
        let Some(sess) = test_session() else { return };
        crate::session_map::note_spawned(&mut sessions, 1, sess);
        sync(&mut sessions, &palette, None);
        assert!(key_matches(&sessions, &palette, None), "first sync banked");
        // Same name, same bg, one resolved color moved.
        let mut moved = palette.clone();
        moved.background = Rgb { r: 1, g: 2, b: 3 };
        assert!(!key_matches(&sessions, &moved, None), "value move re-arms");
        let before = vt_pane::task::frame_gen(sessions.map.get(&1).expect("session"));
        sync(&mut sessions, &moved, None);
        assert!(key_matches(&sessions, &moved, None), "second sync banked");
        let after = vt_pane::task::frame_gen(sessions.map.get(&1).expect("session"));
        assert!(after > before, "palette move invalidated the cached frame");
    }

    #[test]
    fn render_palette_moved_sees_values_not_generations() {
        let palette = theme::builtin::dracula();
        let key = ThemeSyncKey {
            palette: palette.clone(),
            bg: None,
            gen: 7,
        };
        assert!(!render_palette_moved(&key, &palette, None));
        assert!(render_palette_moved(
            &key,
            &palette,
            Some(Rgb { r: 0, g: 0, b: 0 })
        ));
        let mut moved = palette.clone();
        moved.cursor = Rgb { r: 9, g: 8, b: 7 };
        assert!(render_palette_moved(&key, &moved, None));
        // A pure generation bump (spawn/adopt) keeps old caches usable.
        let mut same = key.clone();
        same.gen = 8;
        assert!(!render_palette_moved(&same, &palette, None));
    }
}
