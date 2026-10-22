# Hidden heartbeat + focus floor: minimized busy-loop ~70% -> ~0.4%

## Problem
Two pacing holes surfaced on a 2880x1920@120Hz HiDPI box (release
65728655), initially misread as "xfce components burning CPU" (Xorg,
xfwm4, xfce4-panel and xfdesktop all tracked the terminal's load):

1. **Minimized busy-loop (~70% of a core).** The reactive loop has no
   minimized gate - nothing reads `viewport().minimized`. While the
   window is unmapped, `present` stops blocking on vblank (the only
   real brake the 1ms `SOON_SINGLE_PASS` chain ever had) and egui's
   `predicted_dt` (EMA of frame intervals) inflates across the
   now-long gaps, subtracting any short delay down to "immediate".
   The wake chain degrades into a near-busy render loop: measured
   ~70% app + a matching Xorg/WM ripple while minimized, ~40% while
   visible with a 60Hz TUI output stream.
2. **Unfocused windows stream at the focused 60fps floor.** At 2x
   scale every frame is a full 5760x3840 raster; a background tail
   nobody is reading does not need 60fps.

## Change Summary
1. **`cadence::request_root_after`** (`app/src/render/cadence.rs`):
   the single ROOT request entry - `wake_delay` (single-pass clamp),
   then a hidden clamp to `HIDDEN_HEARTBEAT` (500ms). Every request
   site routes through it: the pty wake closure (`main.rs`),
   `next_deadline` scheduling (`screen.rs`), `request_soon_single_pass`,
   the IME chase loop, and `ipc/server.rs` - which also drops its
   zero-delay double-pass wake for the SOON single-pass one.
2. **Hidden heartbeat**: while minimized, `screen.rs` requests exactly
   the heartbeat (individual deadlines stop mattering). Two passes a
   second keep pty channels drained (unbounded mpsc, but memory is
   not), reconnect/backoff deadlines firing, and the loop alive to
   notice the restore - viewport state only refreshes inside a pass.
   `cadence::observe` records `viewport().minimized/focused` once per
   root pass.
3. **Focus floor** (`pacing_floor`/`floor_for`): an unfocused root
   window paces pty output at `max(floor, 50ms)` (20fps); an explicit
   `TERMINATOR_FRAME_FLOOR_MS` (including `0` = OFF) always wins.

ROOT pacing governs secondaries (they render inside the root pass):
a minimized root with a visible secondary streams at the heartbeat -
rare, and the alternative burned a full core.

## Verification
- `cargo test --workspace` green (2 new cadence tests: hidden-clamp
  collapse/passthrough, unfocused floor raise-never-lower), clippy 0,
  fmt clean.
- Live release build on the affected box (second instance on :0,
  60Hz pty output stream): unfocused 26fps (~4%/5s), focused 53fps
  (~8%), **minimized 2fps (~0.4%)** - old build: minimized ~70%,
  visible ~40%. Idle visible ~0.8%, `pass_avg_us` ~150-200.
