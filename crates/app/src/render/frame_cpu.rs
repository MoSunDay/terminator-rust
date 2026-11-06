//! Process-CPU accounting for the adaptive output frame budget.
//!
//! The reactive loop already renders zero frames when idle, but an
//! output stream (a spinner, an agent log) wakes it every pacing window
//! and the per-frame cost is a property of the scene, not of the
//! schedule: 60fps at ~2.6ms/pass is ~16% of a core no matter how
//! efficiently the pass is written. The only lever that keeps a busy
//! stream inside a CPU budget is the FRAME RATE itself, driven by what a
//! frame actually costs.
//!
//! [`tick`] samples `CLOCK_PROCESS_CPUTIME_ID` (Linux) / `getrusage`
//! (macOS) once per root pass and folds the delta into an 8-sample EMA
//! (50µs..50ms clamp: a first cold frame or a sleep-wake blip must not
//! dominate). `cadence::budgeted_floor` turns that into a pacing floor.
//! Sampling at the START of a pass accounts for the PREVIOUS pass'
//! tessellation + submit, which happen inside eframe after `App::ui`
//! returns.
//!
//! [`mark_interaction`]/[`interaction_age`] latch a 900ms latency window
//! whenever the user touches the app (input event, IPC request, OSC
//! notice, focus gain): the budget never delays the first response to a
//! human, only unattended output. Pure helpers, no classes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// EMA smoothing factor: alpha = 1/8.
pub const EMA_DEN: u64 = 8;
/// Per-sample clamp (µs): below this is timer noise, above is a stall
/// (page-in, a giant paste) that one frame should not pin on the loop.
pub const MIN_SAMPLE_US: u64 = 50;
pub const MAX_SAMPLE_US: u64 = 50_000;
/// Latency window after human input: a 60fps head followed by a capped
/// relaxation tail (see `cadence::interactive_floor`).
pub const INTERACTIVE_MS: u64 = 900;

/// Sentinel for "no previous sample"; a real `CLOCK_PROCESS_CPUTIME_ID`
/// reading is never `u64::MAX` in practice.
const UNSEEN: u64 = u64::MAX;

static LAST_NS: AtomicU64 = AtomicU64::new(UNSEEN);
static EMA_US: AtomicU64 = AtomicU64::new(0);

/// Clock origin for the interaction latch (never zero: 0 = never).
fn interaction_clock() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}

fn interaction_now_ms() -> u64 {
    (interaction_clock()
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64)
        .max(1)
}

static LAST_INTERACTION_MS: AtomicU64 = AtomicU64::new(0);

fn within_interaction(last_ms: u64, now_ms: u64) -> bool {
    last_ms != 0 && now_ms >= last_ms && now_ms - last_ms < INTERACTIVE_MS
}

/// Latch human interaction: the next 900ms of output pacing starts at
/// the 16ms floor and caps the budgeted tail at ~30fps (interactive
/// latency beats the CPU budget, but never paces slower than it).
pub fn mark_interaction() {
    LAST_INTERACTION_MS.store(interaction_now_ms(), Ordering::Relaxed);
}

/// Age of the current human-interaction latch, or `None` once the
/// 900ms window has expired. The age lets the pacing layer keep the
/// first 150ms at full 60fps and relax only the tail of the window.
pub fn interaction_age() -> Option<Duration> {
    let last = LAST_INTERACTION_MS.load(Ordering::Relaxed);
    let now = interaction_now_ms();
    within_interaction(last, now).then(|| Duration::from_millis(now - last))
}

/// One EMA step with the sample clamp. `prev_us == 0` seeds from the
/// first sample instead of dragging the average up from zero.
pub fn ema_step(prev_us: u64, sample_us: u64) -> u64 {
    let sample = sample_us.clamp(MIN_SAMPLE_US, MAX_SAMPLE_US);
    if prev_us == 0 {
        return sample;
    }
    ((prev_us * (EMA_DEN - 1) + sample) / EMA_DEN).clamp(MIN_SAMPLE_US, MAX_SAMPLE_US)
}

/// Last smoothed per-pass CPU cost (µs), 0 before the first sample.
pub fn ema_us() -> u64 {
    EMA_US.load(Ordering::Relaxed)
}

/// Sample process CPU now and fold the delta since the previous call
/// into the EMA. Returns the (possibly unchanged) EMA. Call once at the
/// TOP of every root pass.
pub fn tick() -> u64 {
    let Some(now) = process_cpu_ns() else {
        return ema_us();
    };
    let prev = LAST_NS.swap(now, Ordering::Relaxed);
    if prev == UNSEEN || now <= prev {
        return ema_us();
    }
    let delta_us = (now - prev) / 1_000;
    let next = ema_step(EMA_US.load(Ordering::Relaxed), delta_us);
    EMA_US.store(next, Ordering::Relaxed);
    next
}

#[cfg(target_os = "linux")]
fn process_cpu_ns() -> Option<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out-pointer for the duration of the call.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    if rc != 0 || ts.tv_sec < 0 || ts.tv_nsec < 0 {
        return None;
    }
    Some(
        (ts.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(ts.tv_nsec as u64),
    )
}

#[cfg(target_os = "macos")]
fn process_cpu_ns() -> Option<u64> {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `ru` is a valid out-pointer; RUSAGE_SELF is a constant.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    if rc != 0 {
        return None;
    }
    let secs = (ru.ru_utime.tv_sec as u64).saturating_add(ru.ru_stime.tv_sec as u64);
    let usecs = (ru.ru_utime.tv_usec as u64).saturating_add(ru.ru_stime.tv_usec as u64);
    Some(
        secs.saturating_mul(1_000_000_000)
            .saturating_add(usecs.saturating_mul(1_000)),
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_cpu_ns() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_clamps_every_sample_and_converges() {
        assert_eq!(ema_step(0, 10), MIN_SAMPLE_US);
        assert_eq!(ema_step(0, 1_000_000), MAX_SAMPLE_US);
        let mut ema = ema_step(0, 1_000);
        assert_eq!(ema, 1_000);
        for _ in 0..100 {
            ema = ema_step(ema, 2_000);
        }
        assert!(
            (1_990..=2_000).contains(&ema),
            "8-sample EMA converges to the steady sample ({ema})"
        );
    }

    #[test]
    fn interaction_window_is_bounded() {
        assert!(!within_interaction(0, 10_000), "never interacted");
        assert!(within_interaction(1_000, 1_899), "inside 900ms");
        assert!(!within_interaction(1_000, 1_900), "window ends at 900ms");
        assert!(!within_interaction(2_000, 1_000), "clock went backwards");
    }

    #[test]
    fn interaction_latch_reports_an_age_immediately() {
        mark_interaction();
        let age = interaction_age().expect("latch open");
        assert!(age.as_millis() < INTERACTIVE_MS as u128);
    }
}
