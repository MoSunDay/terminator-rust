# M6: CPU-budget frame governor

## Problem
Reactive rendering removed the idle tick, but unattended output still paced
at `TERMINATOR_FRAME_FLOOR_MS` (default 16ms -> ~60fps). On software
rendering / high-DPI / big windows, a 60Hz spinner can cost tens of percent
of a core forever. The target is <=1% of one core at idle AND at 10Hz output
steady state, with 60fps scenes allowed only via frame-budget downsampling.

## Change Summary
1. **`render/frame_cpu.rs` (new, 177 lines)**: per-pass process CPU sample
   (Linux `CLOCK_PROCESS_CPUTIME_ID`, macOS `getrusage`) folded into an
   alpha=1/8 EMA, sample clamped to 50us..50ms. Human interaction
   (key/pointer/IME/OSC, IPC drain, window focus gain) opens a 900ms
   full-speed latch; `mark_interaction()` is called from `main.rs` input/OSC
   paths, `ipc::server::drain()` and `cadence::observe`.
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

## Verification (release, gl sandbox 1200x800)
- `e2e-idle-cpu.sh` PASS: S1 idle 0 frames / 0.0%; wake 19ms; S7 0.0%;
  S2 24.7fps (startup latch) <= 65; S14 main max1s 1.0%, process 9.0%
  (llvmpipe raster, recorded), fps 1.0; S15 8 panes: avg 0.25%,
  max1s 1.0% (1-2 jiffies), frames 0.
- S14's 1fps is the physical floor on llvmpipe (each frame costs ~60ms of
  rasterizer time; 1% would need 0.16fps). On a real GPU host the per-frame
  process cost is an order of magnitude lower and 10Hz output runs near
  10fps, which is why S14 gates the app main thread in the sandbox and the
  process number on hardware (`measure-cpu-remote.sh s14`).
- `cargo test --workspace` green; fmt clean; clippy baseline only.
