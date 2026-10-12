# Reactive render loop: idle CPU 38% -> 4%

## Problem
The egui loop ran an unconditional repaint tick: 16ms while any pane
had output in the last 500ms, else 50ms - forever. An idle terminal
still burned ~38% of a core (190 utime+stime ticks / 5s, measured on
the old build in `scripts/bin/e2e-idle-cpu.sh`), ~10x every other
terminal, for frames that painted nothing new.

## Change Summary
The loop became event-driven; a repaint is scheduled only when a frame
is actually needed.

1. **PTY wakeups** (`vt-pane/src/task.rs`): `SessionOpts.wake: Option<Wake>`
   (`Wake = Arc<dyn Fn() + Send + Sync>`) fires after every successful
   reader-loop Output/Exit send. The app installs a callback that calls
   `ctx.request_repaint_of(ViewportId::ROOT)`; secondary viewports
   render inside the root pass, so ROOT wakes refresh every window.
2. **IPC wakeups** (`app/src/ipc/server.rs`): the control socket accepts
   with a waker derived from the live `egui::Context`, so `terminator-ctl`
   requests are seen within milliseconds even from a fully idle loop.
3. **Reactive cadence** (`app/src/render/cadence.rs`, new): per frame,
   `next_deadline` collects the EARLIEST future need - cursor-blink edge
   (square wave: 1.2s on / 0.8s off; only the user-focused window
   blinks, others draw a steady cursor), blinking-text edge (533ms
   halves), live-gesture slice (50ms), pane retry/reconnect backoff,
   EXIT_GRACE - and requests exactly that delay. No need = no request:
   a fully idle pane costs zero repaints.
4. **Frame-snapshot cache** (`vt-pane` `Session.cache/cache_valid` +
   `app/src/session_map.rs::sync_frame`): the ghostty snapshot is reused
   across frames and invalidated at every mutating site (pump, resize,
   theme change, selection edits, viewport scroll, follow_output).
   `Frame.has_blink` carries blink dependency so static panes never
   schedule blink repaints.

`terminator-ctl notice` / OSC 9 semantics are untouched: they ride PTY
output, which now wakes the loop directly.

## Verification
- `scripts/bin/e2e-idle-cpu.sh` (new): idle 5s CPU gate (<=100 ticks,
  old build measured 190 = correctly fails), a ctl send->capture marker
  round-trip gate (<=2000ms; measured 52ms), informational busy phase.
  New build: 22 ticks/5s idle (~4.4%), cursor still blinking.
- Full suite re-run green: e2e-notice, e2e-ui-style, e2e-oc-exit,
  e2e-ipc-oc, e2e-mouse-key, e2e-cjk, e2e-bg-seams (K1-K5), plus
  cargo test --workspace (413 tests), fmt, clippy(0), and the
  aarch64-apple-darwin zig cross-check.
- e2e-bg-seams repair (unrelated pre-existing drift): its SGR color
  pins predated the 2026-09-27 theme-palette sync, so bands were
  probed against ghostty-default colors while the engine now (correctly)
  resolves them through the dracula preset. Re-pinned
  red (255,85,85) / blue (189,147,249) / magenta (255,121,198); the
  256-cube orange (255,135,0) is palette-independent and unchanged.
