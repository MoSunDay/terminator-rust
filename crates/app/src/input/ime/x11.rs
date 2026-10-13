//! Repair only a visible XIM panel overlapping the input row, and only while
//! composing and an X11 window belonging to this process owns focus. Checked
//! protocol replies tolerate windows disappearing without a global Xlib error
//! handler.
use std::cell::RefCell;
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

/// Panel placement outcome: `Settled` = nothing to chase, `Verify` = needs
/// one short-delay follow-up frame (a move was issued, the panel is briefly
/// hidden, or nothing measurable was found yet).
#[derive(PartialEq)]
enum Placement {
    Settled,
    Verify,
}

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
fn repair(s: &mut State, local_caret: Rect, composing: bool) -> Result<Placement> {
    // No work at all outside composition (the overlap branch is
    // composing-only too); this also skips the per-frame X round-trips
    // (get_input_focus/pid) on ordinary frames.
    if !composing {
        return Ok(Placement::Settled);
    }
    let focus = s.conn.get_input_focus()?.reply()?.focus;
    if focus == 0 || focus == 1 {
        return Ok(Placement::Settled);
    }
    let pid = s
        .conn
        .get_property(false, focus, s.pid_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?;
    if pid.value32().and_then(|mut v| v.next()) != Some(std::process::id()) {
        return Ok(Placement::Settled);
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
            return Ok(Placement::Verify);
        }
        None => {
            s.candidate = None;
            s.commanded = None;
            if s.last_probe.elapsed() < PROBE_EVERY {
                return Ok(Placement::Verify);
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
            } else {
                // Panel not mapped yet / ambiguous menu: re-check next frame.
                return Ok(Placement::Verify);
            }
        }
    };
    let Some((window, bounds)) = candidate else {
        return Ok(Placement::Verify);
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
        return Ok(Placement::Settled);
    };
    match decide_move(bounds.min, s.commanded, target) {
        None => Ok(Placement::Settled),
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
            Ok(Placement::Verify) // one follow-up frame verifies it landed
        }
    }
}

pub fn sync(ctx: &Context, caret: Rect, composing: bool) {
    if !ctx.input(|i| i.focused) {
        return;
    }
    thread_local! {
        static STATE: RefCell<Option<State>> = RefCell::new(connect());
    }
    STATE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            if let Ok(Placement::Verify) = repair(state, caret * ctx.pixels_per_point(), composing)
            {
                // While composing, chase the panel only while it is missing
                // or misplaced - once settled, input events drive the frames
                // (every keystroke moves the caret); the unconditional 16ms
                // loop fought the IME server at 60Hz and burned CPU.
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
    });
}
