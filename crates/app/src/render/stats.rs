//! Frame-rate instrumentation for the reactive render loop.
//!
//! Env gated (`TERMINATOR_FRAME_STATS=1`): when disabled [`record`] is a
//! single cached branch and costs nothing else. When enabled every root
//! pass folds one sample (pass duration) into an interval accumulator and
//! at most once per second a single line lands on stderr:
//!
//! `frame-stats frames=<n> fps=<fps:.1> pass_avg_us=<avg:.0> pass_max_us=<max>`
//!
//! The math ([`summarize`]/[`format_line`]) is pure and unit-tested; only
//! [`record`] and the two env readers touch process state.
//!
//! The same module owns the output PACING floor used by the pty wake path
//! (`TERMINATOR_FRAME_FLOOR_MS`, default 16ms): a non-zero minimum gap
//! between output-driven frames caps the loop at ~60fps, coalescing
//! bursts of pty output into one frame and leaving headroom on
//! high-refresh displays and Xvfb where the natural cadence would be
//! much faster than the pump can usefully drain.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Output pacing floor (ms) when `TERMINATOR_FRAME_FLOOR_MS` is unset or
/// unusable: ~1/60s between output-driven frames.
pub const DEFAULT_FLOOR_MS: u64 = 16;
/// Upper bound for `TERMINATOR_FRAME_FLOOR_MS`; anything larger would
/// stall visibly (>=1s per frame) and is treated as a typo.
pub const MAX_FLOOR_MS: u64 = 1000;
/// Fallback frame-time prediction (µs) when a pass has no usable
/// `predicted_dt` yet (egui's own default is 1/60s).
pub const PREDICTED_DT_FALLBACK_US: u64 = 16_670;
/// Report cadence: one `frame-stats` line per elapsed second.
pub const REPORT_INTERVAL_MS: u64 = 1000;

/// Fold a window of pass samples into a report. Pure: one line's worth of
/// numbers in, the same numbers (plus derived fps/avg) out; `None` when
/// there is nothing to report (no frames, or a degenerate interval).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub frames: u64,
    pub fps: f64,
    pub pass_avg_us: f64,
    pub pass_max_us: u64,
}

/// Aggregate `frames` root passes observed over `elapsed`, with
/// `pass_sum_us`/`pass_max_us` accumulated independently.
pub fn summarize(
    frames: u64,
    elapsed: Duration,
    pass_sum_us: u64,
    pass_max_us: u64,
) -> Option<Summary> {
    if frames == 0 || elapsed.is_zero() {
        return None;
    }
    Some(Summary {
        frames,
        fps: frames as f64 / elapsed.as_secs_f64(),
        pass_avg_us: pass_sum_us as f64 / frames as f64,
        pass_max_us,
    })
}

/// The exact stderr line shape (stable for log scraping / gates).
pub fn format_line(s: &Summary) -> String {
    format!(
        "frame-stats frames={} fps={:.1} pass_avg_us={:.0} pass_max_us={}",
        s.frames, s.fps, s.pass_avg_us, s.pass_max_us
    )
}

/// Is a new report due for a window that started at `window_ms` and now
/// reads `now_ms` (both ms on the same monotonic origin)?
pub fn report_due(now_ms: u64, window_ms: u64) -> bool {
    now_ms.saturating_sub(window_ms) >= REPORT_INTERVAL_MS
}

/// `TERMINATOR_FRAME_FLOOR_MS` -> milliseconds. Unparseable, zero or
/// absurd (> [`MAX_FLOOR_MS`]) values fall back to [`DEFAULT_FLOOR_MS`].
pub fn parse_floor_ms(s: &str) -> u64 {
    s.trim()
        .parse::<u64>()
        .ok()
        .filter(|v| (1..=MAX_FLOOR_MS).contains(v))
        .unwrap_or(DEFAULT_FLOOR_MS)
}

/// f32 seconds -> microseconds for the pacing math, guarding non-finite,
/// non-positive and absurd values (>= 1s/frame) with the fallback.
pub fn predicted_dt_us(secs: f32) -> u64 {
    let us = f64::from(secs) * 1e6;
    if !us.is_finite() || !(1.0..=1e6).contains(&us) {
        return PREDICTED_DT_FALLBACK_US;
    }
    (us.round().max(1.0) as u64).min(1_000_000)
}

/// Cheap gate for [`record`]: read once, then a cached branch per pass.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TERMINATOR_FRAME_STATS").is_ok_and(|v| v == "1"))
}

/// The output pacing floor as a duration (see module docs).
pub fn output_floor() -> Duration {
    static FLOOR_MS: OnceLock<u64> = OnceLock::new();
    let ms = *FLOOR_MS.get_or_init(|| {
        std::env::var("TERMINATOR_FRAME_FLOOR_MS").map_or(DEFAULT_FLOOR_MS, |v| parse_floor_ms(&v))
    });
    Duration::from_millis(ms)
}

/// Monotonic origin shared by the interval clock (first use wins).
fn start_instant() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

/// Milliseconds since [`start_instant`].
fn now_ms() -> u64 {
    start_instant().elapsed().as_millis().min(u64::MAX as u128) as u64
}

// Window bookkeeping: start of the current reporting interval, ms on
// the same monotonic origin as `record`'s clock.
static WINDOW_START_MS: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static PASS_SUM_US: AtomicU64 = AtomicU64::new(0);
static PASS_MAX_US: AtomicU64 = AtomicU64::new(0);

/// Record one root pass (its measured duration). No-op unless
/// `TERMINATOR_FRAME_STATS=1`; when enabled it may emit the report line,
/// at most once per [`REPORT_INTERVAL_MS`]. Single caller (the root pass
/// on the UI thread), so plain relaxed atomics suffice.
pub fn record(pass: Duration) {
    if !enabled() {
        return;
    }
    let pass_us = pass.as_micros().min(u64::MAX as u128) as u64;
    let now = now_ms();
    FRAMES.fetch_add(1, Ordering::Relaxed);
    PASS_SUM_US.fetch_add(pass_us, Ordering::Relaxed);
    PASS_MAX_US.fetch_max(pass_us, Ordering::Relaxed);
    let window = WINDOW_START_MS.load(Ordering::Relaxed);
    if report_due(now, window) {
        WINDOW_START_MS.store(now, Ordering::Relaxed);
        let frames = FRAMES.swap(0, Ordering::Relaxed);
        let sum_us = PASS_SUM_US.swap(0, Ordering::Relaxed);
        let max_us = PASS_MAX_US.swap(0, Ordering::Relaxed);
        let elapsed = Duration::from_millis(now.saturating_sub(window));
        if let Some(s) = summarize(frames, elapsed, sum_us, max_us) {
            eprintln!("{}", format_line(&s));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_env_values_are_guarded() {
        assert_eq!(parse_floor_ms("16"), 16);
        assert_eq!(parse_floor_ms("1"), 1);
        assert_eq!(parse_floor_ms("1000"), MAX_FLOOR_MS);
        assert_eq!(parse_floor_ms(" 20 "), 20);
        // Unset is handled by the caller; bad values map to the default.
        assert_eq!(parse_floor_ms(""), DEFAULT_FLOOR_MS);
        assert_eq!(parse_floor_ms("abc"), DEFAULT_FLOOR_MS);
        assert_eq!(parse_floor_ms("0"), DEFAULT_FLOOR_MS);
        assert_eq!(parse_floor_ms("-3"), DEFAULT_FLOOR_MS);
        assert_eq!(parse_floor_ms("3.5"), DEFAULT_FLOOR_MS);
        assert_eq!(parse_floor_ms("5000"), DEFAULT_FLOOR_MS);
    }

    #[test]
    fn predicted_dt_is_clamped_to_micros() {
        assert_eq!(predicted_dt_us(1.0 / 60.0), 16_667);
        assert_eq!(predicted_dt_us(0.0167), 16_700);
        // Non-finite / non-positive / absurd -> the fallback.
        assert_eq!(predicted_dt_us(f32::NAN), PREDICTED_DT_FALLBACK_US);
        assert_eq!(predicted_dt_us(f32::INFINITY), PREDICTED_DT_FALLBACK_US);
        assert_eq!(predicted_dt_us(0.0), PREDICTED_DT_FALLBACK_US);
        assert_eq!(predicted_dt_us(-0.5), PREDICTED_DT_FALLBACK_US);
        assert_eq!(predicted_dt_us(2.0), PREDICTED_DT_FALLBACK_US);
    }

    #[test]
    fn summarize_folds_a_one_second_window() {
        // 60 frames over 1s, 10ms average, 20ms worst.
        let s = summarize(60, Duration::from_secs(1), 600_000, 20_000).expect("summary");
        assert_eq!(s.frames, 60);
        assert!((s.fps - 60.0).abs() < 1e-9);
        assert!((s.pass_avg_us - 10_000.0).abs() < 1e-6);
        assert_eq!(s.pass_max_us, 20_000);
    }

    #[test]
    fn summarize_rejects_empty_windows() {
        assert!(summarize(0, Duration::from_secs(1), 0, 0).is_none());
        assert!(summarize(60, Duration::ZERO, 600_000, 20_000).is_none());
    }

    #[test]
    fn report_line_is_stable() {
        let s = summarize(120, Duration::from_millis(2000), 2_400_000, 41_000).expect("summary");
        assert_eq!(
            format_line(&s),
            "frame-stats frames=120 fps=60.0 pass_avg_us=20000 pass_max_us=41000"
        );
    }

    #[test]
    fn reports_land_once_per_interval() {
        assert!(report_due(1000, 0));
        assert!(report_due(2500, 1000));
        assert!(!report_due(999, 0));
        assert!(!report_due(1500, 1000));
        // A clock that reads before the window start never reports.
        assert!(!report_due(10, 1000));
    }
}
