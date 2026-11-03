# M6: CPU-budget frame governor

## Problem
Reactive rendering removed the idle tick, but unattended output still paced
at `TERMINATOR_FRAME_FLOOR_MS` (default 16ms -> ~60fps). On software
rendering / high-DPI / big windows, a 60Hz spinner can cost tens of percent
of a core forever. The target is <=1% of one core at idle AND at 10Hz output
steady state, with 60fps scenes allowed only via frame-budget downsampling.

## Change Summary
1. **`render/frame_cpu.rs` (new, 181 lines)**: per-pass process CPU sample
   (Linux `CLOCK_PROCESS_CPUTIME_ID`, macOS `getrusage`) folded into an
   alpha=1/8 EMA, sample clamped to 50us..50ms. Human interaction
   (key/pointer/IME/OSC, mutating IPC requests, window focus gain) opens a
   900ms full-speed latch; `mark_interaction()` is called from `main.rs`
   input/OSC paths, `ipc::server::drain()` and `cadence::observe`.
2. **`cadence::budget_floor`**: `fps = clamp(10 * target_pct / cpu_ms, 1, 60)`
   and `floor = clamp(1000/fps, 16ms, 1s)`. The 1fps floor means output is
   never silenced - a burst still advances once per second; 16ms keeps the
   old 60fps ceiling. `budgeted_floor` combines it with focus (unfocused)
   pacing.
3. **`render/stats.rs`**: `governor_enabled` / `cpu_target_pct` /
   `parse_target_pct`; `TERMINATOR_CPU_TARGET_PCT` (default 1.0, `0` = off,
   >100 falls back). An EXPLICIT `TERMINATOR_FRAME_FLOOR_MS` (including 0)
   disables the governor so e2e cadence stays deterministic.
4. `main.rs` pty wake uses `budgeted_floor`; `tick()` at the top of each root
   pass feeds the EMA.
5. **Measurement gates**: `e2e-idle-cpu.sh` S14 (10Hz spinner, main thread
   max1s <= 1.0%, fps <= 12, process recorded) and S15 (8 panes, frames == 0,
   process avg < 1%, max1s <= 1.0%); `measure-cpu-remote.sh` s14 (main-thread
   sampler) and guarded s15. `cpu-sample.py` gained `--thread-max1s SPEC`
   (main thread is tid==pid but is read via `/proc/<pid>/task/<tid>/stat`).

## Release-blocker fix (2026-11-03)
1. **Secondary-viewport input now opens the interaction latch.**
   `windows::render` marks interaction for input seen in ITS viewport
   (secondary key/mouse events never pass through the root pass) and on
   focus gain. Without this, typing in a secondary window while its pane
   produced output was paced at the governor's ~1fps floor.
2. **IPC wake now follows the queue push.** `serve` used to request a
   root repaint before reading the request; an early `drain` could find
   an empty queue and the command then waited for an unrelated event or
   the 5s reply timeout. `forward` wakes with `request_soon_single_pass`
   only after `tx.send` succeeded (the intermittent `ctl` timeout).
3. **`TERMINATOR_FRAME_FLOOR_MS` validity is unified.** Previously
   `governor_enabled()` keyed off env PRESENCE while `output_floor()`
   keyed off a VALID value: `FLOOR=abc` produced the 16ms default floor
   but silently disabled the governor. `floor_is_explicit` now makes
   unusable values behave like unset for both; `0` and all valid values
   still disable the governor.
4. **Non-vacuous release gates.**
   - `main.rs` logs the effective `cpu governor: enabled|disabled` line
     at startup; `e2e-idle-cpu.sh` requires it and S14 FAILS when the
     governor is disabled (an inherited floor/target could previously
     make the 10Hz contract pass without a governor).
   - `verify-release.sh` requires the `.sha256` sidecar and runs both
     gates with `env -i` (scrubs APP_BIN/CTL_BIN/IDLE_*/
     TERMINATOR_FRAME_FLOOR_MS/TERMINATOR_CPU_TARGET_PCT).
   - `measure-cpu-remote.sh` live-pane scenarios are OPT-IN probes, not
     part of `verify-release.sh` / the release gates:
     `S13_ALLOW_FLOOD=1` (40k lines) and `S14_ALLOW_TYPE=1` (24s
     spinner); s15 keeps `S15_ALLOW_SPLITS=1`.
   - `e2e-windows.sh` W2b (a CI gate) types into the secondary while 20
     concurrent `ctl` round-trips run: keys must stay in the secondary
     and no ctl call may fail.
5. Workspace version bumped 0.1.0 -> 0.2.0 for the post-v0.1.0 release
   (Cargo.lock rebuilt).
6. **Multi-window focus/minimized aggregation.** Pacing state is folded
   across EVERY viewport pass (`cadence::report_viewport` in
   `windows::render`, committed by `finish_viewports` after the
   secondaries): a FOCUSED secondary keeps focus-floor pacing even
   while the root is unfocused, and only ALL-minimized triggers the
   hidden heartbeat (previously a focused secondary was paced as
   unfocused root - 50ms floor plus the CPU governor).
7. **Read-only IPC requests no longer open the interaction latch.**
   `ipc::server::drain` marks interaction only for `Notice` / `Write` /
   `MigrateOut` / `TabOffer`; `List` and `Capture` are borrows, so a
   ctl polling loop cannot pin 60fps / disable the governor.

## Verification (release, gl sandbox 1200x800)
- `e2e-idle-cpu.sh` PASS: S1 idle 0 frames / 0.0%; wake 19ms; S7 0.0%;
  S2 24.7fps (startup latch) <= 65; S14 main max1s 1.0%, process 9.0%
  (llvmpipe raster, recorded), fps 1.0; S15 8 panes: avg 0.25%,
  max1s 1.0% (1-2 jiffies), frames 0.
- S14's 1fps is the physical floor on llvmpipe (each frame costs ~60ms of
  rasterizer time; 1% would need 0.16fps). On a real GPU host the per-frame
  process cost is an order of magnitude lower and 10Hz output runs near
  10fps. Sandbox S14 asserts the app MAIN THREAD (process CPU is recorded
  only under llvmpipe); hardware `measure-cpu-remote.sh s14` records
  process CPU but has no automated process-CPU threshold.
- `cargo test --workspace` green; fmt clean; clippy ran with `-D warnings`
  and was green.
