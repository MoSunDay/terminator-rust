//! Repaint scheduling for the reactive render loop.
//!
//! The app used to request a repaint every 16/50 ms unconditionally,
//! keeping every window rendering at 20-60 fps even when nothing changed
//! (an idle terminal burned CPU forever). This module replaces that tick
//! with the EARLIEST deadline that actually needs a frame; when nothing
//! needs one, no repaint is requested at all and eframe sleeps until
//! input, a pty-reader wake (`SessionOpts::wake`) or an IPC request
//! arrives.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use layout_tree::PaneId;

use crate::render::tokens::{CURSOR_ON_S, CURSOR_PERIOD_S};
use crate::session_map::{SessionMap, EXIT_GRACE};

/// Text-blink duty cycle; mirrors the `blink_on` phase in screen.rs.
pub const TEXT_BLINK_PERIOD_S: f32 = 1.0;
pub const TEXT_BLINK_ON_S: f32 = 0.5;

/// While a drag/gesture is mid-flight (pane-move dwell, tab drag, edge
/// resize), keep frames coming so purely time-driven steps (dwell
/// switching) advance even with a still pointer.
const INTERACT_SLICE: Duration = Duration::from_millis(50);

/// What the current pass still needs a future frame for.
#[derive(Debug, Default, Clone, Copy)]
pub struct Needs {
    /// The user-focused window's focused pane draws a visible, blinking
    /// cursor (not hidden by IME composition).
    pub cursor_blink: bool,
    /// Some visible pane carries blink-attribute text.
    pub blink_text: bool,
    /// A drag/gesture is mid-flight.
    pub interacting: bool,
}

/// Pseudo-immediate single-pass wake: NON-ZERO at the
/// `request_repaint_after_for` call site, so egui 0.36 does NOT arm the
/// zero-delay double pass - context.rs only arms `outstanding = 1` for
/// `delay == Duration::ZERO` ("Each request results in two repaints,
/// just to give some things time to settle"); for non-zero delays "we
/// only repaint once". egui then subtracts the viewport's `predicted_dt`
/// (~16.7ms) AFTER that decision, so this saturates to an immediate
/// single pass: one frame, right away, no wasted second pass.
pub const SOON_SINGLE_PASS: Duration = Duration::from_millis(1);

/// Wake delay for pty output: pace frames to at most one per `floor`
/// (`elapsed` = time since the last frame started, `predicted_dt` =
/// egui's estimate of one frame).
///
/// - `elapsed >= floor`: the floor already passed, so wake with
///   [`SOON_SINGLE_PASS`] - one immediate pass, no double pass.
/// - otherwise: wait the REMAINDER of the floor plus `predicted_dt`,
///   because egui subtracts `predicted_dt` from the requested delay
///   before scheduling (context.rs `request_repaint_after`: "Make it
///   less likely we over-shoot the target"). The subtraction lands the
///   real wake ~`floor - elapsed` from now, and the sum is never zero,
///   so this stays a single-pass REAL wait - capping the loop at
///   ~1/floor (60fps at the 16ms default) instead of one frame per
///   output chunk.
pub fn output_wake_paced(elapsed: Duration, predicted_dt: Duration, floor: Duration) -> Duration {
    if elapsed >= floor {
        SOON_SINGLE_PASS
    } else {
        floor - elapsed + predicted_dt
    }
}

/// Ask for a ROOT repaint "as soon as possible, but single-pass": a
/// [`SOON_SINGLE_PASS`] delay instead of a zero-delay
/// `request_repaint()` (which egui turns into TWO passes). Secondary
/// viewports render inside the root pass, so waking ROOT refreshes every
/// window.
pub fn request_soon_single_pass(ctx: &egui::Context) {
    ctx.request_repaint_after_for(SOON_SINGLE_PASS, egui::ViewportId::ROOT);
}

/// Seconds until the next on/off edge of a duty-cycle square wave whose
/// clock reads `t` (seconds since process start).
fn next_edge(t: f32, period: f32, on: f32) -> Duration {
    let phase = t.rem_euclid(period);
    let until = if phase < on {
        on - phase
    } else {
        period - phase
    };
    Duration::from_secs_f32(until.max(0.0))
}

/// Earliest of a per-pane instant map (offset added to each entry) as a
/// delay from `now`; `Duration::ZERO` when already overdue.
fn min_map(m: &HashMap<PaneId, Instant>, offset: Duration, now: Instant) -> Option<Duration> {
    m.values()
        .map(|t| (*t + offset).saturating_duration_since(now))
        .min()
}

/// The next wakeup the UI needs, or `None` when truly idle. Deadlines
/// collected: cursor-blink and blink-text edges, spawn backoff
/// (`retry_at`), reconnect backoff (`reconnect_at`) and the exited-pane
/// auto-close grace (`exited_seen` + [`EXIT_GRACE`]).
pub fn next_deadline(
    now: Instant,
    now_s: f32,
    sess: &SessionMap,
    needs: Needs,
) -> Option<Duration> {
    let mut out: Option<Duration> = None;
    let keep_min = |a: Option<Duration>, b: Option<Duration>| match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    };
    if needs.interacting {
        out = Some(INTERACT_SLICE);
    }
    if needs.cursor_blink {
        out = keep_min(out, Some(next_edge(now_s, CURSOR_PERIOD_S, CURSOR_ON_S)));
    }
    if needs.blink_text {
        out = keep_min(
            out,
            Some(next_edge(now_s, TEXT_BLINK_PERIOD_S, TEXT_BLINK_ON_S)),
        );
    }
    out = keep_min(out, min_map(&sess.retry_at, Duration::ZERO, now));
    out = keep_min(out, min_map(&sess.reconnect_at, Duration::ZERO, now));
    out = keep_min(out, min_map(&sess.exited_seen, EXIT_GRACE, now));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> SessionMap {
        crate::session_map::session_map()
    }

    #[test]
    fn idle_without_needs_or_deadlines_is_none() {
        assert!(next_deadline(Instant::now(), 0.0, &empty(), Needs::default()).is_none());
    }

    /// Milliseconds, rounded: f32 duty math drifts a few 100ns, so edge
    /// asserts compare at ~1ms resolution instead of exact equality.
    fn close(actual: Duration, want_ms: i64) {
        let got = actual.as_millis() as i64;
        assert!(
            (got - want_ms).abs() <= 2,
            "edge {actual:?} ~= {want_ms}ms (got {got}ms)"
        );
    }

    #[test]
    fn blink_edges_land_on_duty_boundaries() {
        // Cursor: 1.2s on / 0.8s off per 2s.
        close(next_edge(0.0, 2.0, 1.2), 1200);
        close(next_edge(1.2, 2.0, 1.2), 800);
        close(next_edge(1.9, 2.0, 1.2), 100);
        // Text: 0.5s on / 0.5s off per 1s.
        close(next_edge(0.2, 1.0, 0.5), 300);
        close(next_edge(0.6, 1.0, 0.5), 400);
        // Negative clocks stay well-defined (rem_euclid).
        close(next_edge(-1.3, 2.0, 1.2), 500);
    }

    #[test]
    fn picks_the_earliest_need() {
        let now = Instant::now();
        let needs = Needs {
            cursor_blink: true,
            blink_text: true,
            interacting: false,
        };
        let d = next_deadline(now, 0.25, &empty(), needs).expect("some deadline");
        assert_eq!(d, Duration::from_millis(250)); // text edge < cursor edge

        // A live gesture caps the wait at the interaction slice.
        let live = Needs {
            interacting: true,
            ..needs
        };
        assert_eq!(
            next_deadline(now, 0.25, &empty(), live),
            Some(INTERACT_SLICE)
        );
    }

    #[test]
    fn backoff_and_grace_maps_contribute_deadlines() {
        let mut sess = empty();
        let now = Instant::now();
        sess.retry_at.insert(1, now + Duration::from_millis(150));
        sess.reconnect_at
            .insert(2, now + Duration::from_millis(300));
        // seen 200ms ago + EXIT_GRACE(250ms) -> due in 50ms: earliest.
        sess.exited_seen.insert(3, now - Duration::from_millis(200));
        let d = next_deadline(now, 0.0, &sess, Needs::default()).expect("some deadline");
        assert_eq!(d, Duration::from_millis(50));
    }

    #[test]
    fn overdue_instant_clamps_to_immediate() {
        let mut sess = empty();
        let now = Instant::now();
        sess.retry_at.insert(7, now - Duration::from_secs(1));
        let d = next_deadline(now, 0.0, &sess, Needs::default()).expect("some deadline");
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn output_wake_waits_out_the_floor() {
        let floor = Duration::from_millis(16);
        let pred = Duration::from_micros(16_670); // 1/60s, egui's default
                                                  // A frame just rendered: the requested delay adds predicted_dt
                                                  // back on top of the remaining floor, because egui subtracts it.
        assert_eq!(
            output_wake_paced(Duration::ZERO, pred, floor),
            Duration::from_micros(32_670)
        );
        // egui's real default prediction (1/60s), from mid-floor: ~28.67ms.
        let pred60 = Duration::from_secs_f64(1.0 / 60.0);
        let d = output_wake_paced(Duration::from_millis(4), pred60, floor);
        let ms = d.as_secs_f64() * 1e3;
        assert!(
            (ms - 28.67).abs() < 0.01,
            "mid-floor delay {ms}ms ~= 28.67ms"
        );
        // A real wait: never zero, so egui never arms the double pass.
        assert!(!d.is_zero());
    }

    #[test]
    fn output_wake_is_immediate_once_the_floor_elapsed() {
        let floor = Duration::from_millis(16);
        let pred = Duration::from_millis(17);
        // At, past and long past the floor: one pseudo-immediate pass.
        assert_eq!(output_wake_paced(floor, pred, floor), SOON_SINGLE_PASS);
        assert_eq!(
            output_wake_paced(Duration::from_millis(20), pred, floor),
            Duration::from_millis(1)
        );
        assert_eq!(
            output_wake_paced(Duration::from_secs(5), pred, floor),
            Duration::from_millis(1)
        );
        assert!(!SOON_SINGLE_PASS.is_zero());
    }
}
