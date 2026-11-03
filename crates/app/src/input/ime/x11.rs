//! Repair only a visible XIM panel overlapping the input row, and only while
//! composing and an X11 window belonging to this process owns focus. Checked
//! protocol replies tolerate windows disappearing without a global Xlib error
//! handler.
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use anyhow::Result;
use egui::{pos2, vec2, Context, Pos2, Rect};
use x11rb::connection::Connection;
use x11rb::protocol::{randr, xproto::*};
use x11rb::rust_connection::RustConnection;

use super::popup::{avoid_row, decide_move, follow_row};

/// How long the cached candidate identity survives the panel measuring as
/// non-VIEWABLE: the IME briefly unmaps it while redrawing the list.
const CANDIDATE_GRACE: Duration = Duration::from_millis(150);

/// Full-tree probe throttle: scanning every root child costs a round-trip
/// per window, so misses re-probe at most every 50ms.
const PROBE_EVERY: Duration = Duration::from_millis(50);

struct State {
    conn: RustConnection,
    root: Window,
    pid_atom: Atom,
    candidate: Option<Window>,
    last_probe: Instant,
    /// Last root-px position WE commanded for `candidate` (None until the
    /// first move) - the idempotence latch input.
    commanded: Option<Pos2>,
    /// When `candidate` last measured as a visible panel.
    last_seen: Instant,
    classes: &'static [&'static str],
}

fn connect() -> Option<State> {
    let modifiers = std::env::var("XMODIFIERS").ok()?;
    let classes: &[&str] = match modifiers.as_str() {
        "@im=ibus" => &["ibus-ui-gtk3"],
        "@im=fcitx" => &["fcitx", "fcitx5"],
        _ => return None,
    };
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen].root;
    let pid_atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()?
        .reply()
        .ok()?
        .atom;
    Some(State {
        conn,
        root,
        pid_atom,
        candidate: None,
        last_probe: Instant::now(),
        commanded: None,
        last_seen: Instant::now(),
        classes,
    })
}

fn panel(s: &State, window: Window) -> Result<Option<Rect>> {
    let attrs = s.conn.get_window_attributes(window)?.reply()?;
    if attrs.map_state != MapState::VIEWABLE || !attrs.override_redirect {
        return Ok(None);
    }
    let class = s
        .conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 128)?
        .reply()?;
    let matches = class.value.split(|b| *b == 0).any(|name| {
        s.classes
            .iter()
            .any(|class| name.eq_ignore_ascii_case(class.as_bytes()))
    });
    if !matches {
        return Ok(None);
    }
    let geo = s.conn.get_geometry(window)?.reply()?;
    let at = s
        .conn
        .translate_coordinates(window, s.root, 0, 0)?
        .reply()?;
    Ok(Some(Rect::from_min_size(
        pos2(f32::from(at.dst_x), f32::from(at.dst_y)),
        vec2(f32::from(geo.width), f32::from(geo.height)),
    )))
}

/// Monitor containing `caret` (root px); `None` when RandR lists none.
fn monitor_of(s: &State, caret: &Rect) -> Result<Option<Rect>> {
    let monitors = randr::get_monitors(&s.conn, s.root, true)?.reply()?;
    Ok(monitors
        .monitors
        .iter()
        .map(|m| {
            Rect::from_min_size(
                pos2(f32::from(m.x), f32::from(m.y)),
                vec2(f32::from(m.width), f32::from(m.height)),
            )
        })
        .find(|m| m.contains(caret.center())))
}

/// Chase the panel toward the caret row while composing. Purely idempotent
/// once settled: a move is issued only when `decide_move` says the panel is
/// not where we last put it (or the target genuinely moved), so the IME
/// server's own repositioning no longer starts a per-frame tug-of-war.
/// Returns the panel-vanish cancel signal: `true` = the chased panel
/// vanished while composing (see the `None =>` arm below).
fn repair(s: &mut State, local_caret: Rect, composing: bool) -> Result<bool> {
    // No work at all outside composition (the overlap branch is
    // composing-only too); this also skips the per-frame X round-trips
    // (get_input_focus/pid) on ordinary frames. Clearing the chase here
    // resets the panel identity between compositions so a stale window
    // never reports a vanish on the next one.
    if !composing {
        s.candidate = None;
        s.commanded = None;
        return Ok(false);
    }
    let focus = s.conn.get_input_focus()?.reply()?.focus;
    if focus == 0 || focus == 1 {
        return Ok(false);
    }
    let pid = s
        .conn
        .get_property(false, focus, s.pid_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?;
    if pid.value32().and_then(|mut v| v.next()) != Some(std::process::id()) {
        return Ok(false);
    }
    let at = s.conn.translate_coordinates(focus, s.root, 0, 0)?.reply()?;
    let caret = local_caret.translate(vec2(f32::from(at.dst_x), f32::from(at.dst_y)));
    let cached = s
        .candidate
        .and_then(|w| panel(s, w).ok().flatten().map(|p| (w, p)));
    let candidate = match cached {
        Some(found) => {
            s.last_seen = Instant::now();
            Some(found)
        }
        // Briefly unmapped while the IME redraws candidates: keep the
        // identity and re-check on the next frame.
        None if s.last_seen.elapsed() < CANDIDATE_GRACE => {
            return Ok(false);
        }
        None => {
            let had = s.candidate.is_some();
            s.candidate = None;
            s.commanded = None;
            if s.last_probe.elapsed() < PROBE_EVERY {
                return Ok(false);
            }
            s.last_probe = Instant::now();
            let windows = s.conn.query_tree(s.root)?.reply()?.children;
            let candidates: Vec<_> = windows
                .into_iter()
                .filter_map(|w| panel(s, w).ok().flatten().map(|p| (w, p)))
                .collect();
            // Ambiguous panel ownership (e.g. an input-method menu is open): leave
            // those windows alone rather than moving an unrelated popup.
            if candidates.len() == 1 {
                s.last_seen = Instant::now();
                candidates.into_iter().next()
            } else if candidates.is_empty() && had {
                // The panel we were chasing is unmapped and nothing took
                // its place: the server cancelled the composition
                // silently (Escape/BackSpace empty preedit fires no
                // preedit-done); this disappearance is the only cancel
                // signal the client gets.
                return Ok(true);
            } else {
                // Panel not mapped yet / ambiguous menu: re-check next frame.
                return Ok(false);
            }
        }
    };
    let Some((window, bounds)) = candidate else {
        return Ok(false);
    };
    s.candidate = Some(window);
    // Both branches are composing-only by construction (the guard above);
    // a panel parked far from the caret row (XIM servers that ignore the
    // spot under preedit callbacks, libX11 bug 1580) is pulled next to it.
    let monitor = monitor_of(s, &caret)?;
    let target = if bounds.bottom() > caret.top() && bounds.top() < caret.bottom() {
        monitor.and_then(|m| avoid_row(caret, bounds, m))
    } else {
        monitor.and_then(|m| follow_row(caret, bounds, m))
    };
    let Some(target) = target else {
        return Ok(false);
    };
    match decide_move(bounds.min, s.commanded, target) {
        None => Ok(false),
        Some(at) => {
            s.conn
                .configure_window(
                    window,
                    &ConfigureWindowAux::new()
                        .x(at.x.round() as i32)
                        .y(at.y.round() as i32),
                )?
                .check()?;
            s.conn.flush()?;
            s.commanded = Some(at);
            Ok(false)
        }
    }
}

// Force a real X FocusOut/FocusIn pair for the focused top-level window
// owned by this process. winit's X11 backend XUnsetICFocus/XSetICFocus the
// SAME ic from its XI FocusOut/FocusIn handlers, and that pair is what
// resets the ibus server-side preedit state after a commit - recreating
// the context instead wedges the server (see ime::restart_after_commit).
thread_local! {
    /// Window whose focus the bounce is holding (set by the FocusOut half).
    static BOUNCE: std::cell::Cell<Window> = const { std::cell::Cell::new(0) };
}

fn focused_self() -> Result<Option<Window>> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let pid_atom = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
    let win = conn.get_input_focus()?.reply()?.focus;
    if win == 0 || win == 1 || win == root {
        return Ok(None);
    }
    // Never bounce another process's window (focus can race a switch).
    let pid = conn
        .get_property(false, win, pid_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?;
    Ok((pid.value32().and_then(|mut v| v.next()) == Some(std::process::id())).then_some(win))
}

fn set_focus(conn: &RustConnection, win: Window) -> Result<()> {
    conn.set_input_focus(InputFocus::PARENT, win, Time::CURRENT_TIME)?
        .check()?;
    conn.flush()?;
    Ok(())
}

/// FocusOut half of the XIM re-arm: move focus to the root while remembering
/// this process's focused top-level window. Returns true when the bounce was
/// actually started (a focused self was found and focus moved to root);
/// false when this process holds no focused window - nothing to bounce, and
/// the eventual FocusIn arms the ic anyway.
pub(crate) fn begin_focus_bounce() -> bool {
    (|| -> Result<bool> {
        let Some(win) = focused_self()? else {
            return Ok(false);
        };
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        set_focus(&conn, root)?;
        BOUNCE.with(|b| b.set(win));
        Ok(true)
    })()
    .unwrap_or(false)
}

/// FocusIn half: winit's X11 backend only calls XSetICFocus from this event,
/// which is what (re)arms the ic the bounce defocused.
pub(crate) fn finish_focus_bounce() {
    let _ = (|| -> Result<()> {
        let win = BOUNCE.with(Cell::get);
        if win == 0 {
            return Ok(());
        }
        BOUNCE.with(|b| b.set(0));
        let (conn, _) = x11rb::connect(None)?;
        set_focus(&conn, win)
    })();
}

/// One pass of the panel chase (see [`repair`]). Returns the panel-vanish
/// cancel signal: `true` when the chased candidate panel unmapped
/// mid-composition with nothing taking its place - only meaningful while
/// `composing` (outside it the chase is reset and never reports).
pub fn sync(ctx: &Context, caret: Rect, composing: bool) -> bool {
    if !ctx.input(|i| i.focused) {
        return false;
    }
    thread_local! {
        static STATE: RefCell<Option<State>> = RefCell::new(connect());
    }
    STATE.with(|state| {
        let vanished = state.borrow_mut().as_mut().is_some_and(|st| {
            repair(st, caret * ctx.pixels_per_point(), composing).unwrap_or(false)
        });
        // The panel chase must run for the WHOLE composition: the IME
        // server re-docks its panel asynchronously after its own
        // triggers (every candidate update), and none of that wakes a
        // frame - stopping at the first "settled" observation left the
        // panel parked when the server's last move landed after ours
        // (release-build timing exposed it; the idempotence latch keeps
        // each pass cheap: no XMoveWindow while genuinely in place).
        // Outside composition `repair` early-returns and no loop runs.
        if composing {
            crate::render::cadence::request_root_after(ctx, Duration::from_millis(16));
        }
        vanished
    })
}
