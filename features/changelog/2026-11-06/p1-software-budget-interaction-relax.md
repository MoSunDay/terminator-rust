# P1: software-adapter budget, interaction relax, present probe

## Problem
The reactive loop was idle-clean, but output-driven frames still paid the
full software-raster bill. On llvmpipe the M6 governor landed at its 1fps
floor (each frame ~60ms of rasterizer work) yet the target remained 1% of
one core, so the process average sat around 9% even though the app loop
itself was well under 1%. Interaction was all-or-nothing too: any key or
pointer event held the 900ms latch at 60fps, including its long tail after
the user had stopped typing. The e2e gate also asserted a single 1-second
bucket, which is quantized by the kernel's 100Hz tick.

## Change Summary
1. **Software-adapter target** (`crates/app/src/render/stats.rs`):
   `SOFTWARE_CPU_TARGET_PCT = 0.3`, selected when the adapter name looks
   like llvmpipe / softpipe / SwiftShader / lavapipe / WARP / Microsoft
   Basic Render Driver / Apple Software Renderer, or wgpu's
   `DeviceType::Cpu`. An explicit `TERMINATOR_CPU_TARGET_PCT` still wins,
   including `0` (governor off); an invalid value is discarded and logged
   as unset-like instead of pinning the hardware default. Detection runs
   in `Terminator::new` after eframe exposes the adapter; the startup
   line reports the effective target AND its source (explicit env /
   software adapter / default). At the 1fps software floor the target does not
   change the frame count; the gain is the 3.3x longer budget interval
   while a frame is still cheap enough to stay above that floor.
2. **Interaction tail relax** (`crates/app/src/render/frame_cpu.rs`,
   `cadence.rs`): `interaction_age()` replaces the boolean latch.
   The first `INTERACTIVE_FULL_MS = 150` stays at 60fps; after that, an
   over-budget frame's governed floor is capped at
   `INTERACTIVE_RELAXED_FLOOR = 33ms` (~30fps) - a cap, never a slower
   replacement. Cheap frames keep 60fps for the whole latch, and an off
   governor never throttles input.
3. **Present-mode experiment** (`crates/app/src/render/present.rs`):
   `TERMINATOR_PRESENT_MODE` selects an alternate surface mode
   (`immediate`/`auto-no-vsync`, `mailbox`, `fifo`, `fifo-relaxed`) with
   one frame of latency; unset/invalid keeps egui-wgpu's
   `HIGH_THROUGHPUT` preset. Raw Mailbox/FifoRelaxed requests are mapped
   to wgpu's `Auto*` fallback chains because an unsupported raw mode
   panics in `Surface::configure`; the log shows the requested and
   effective Auto mode. This is a measurement hook, not a setting.
4. **Process-level e2e gates** (`scripts/bin/e2e-idle-cpu.sh`):
   under the gl/software backend S2 asserts process average < 80% and
   S14 asserts process average < 30%; native only records. S14's main
   thread contract is now average < 1.0% plus a max1s <= 2.0% burst
   ceiling. `/proc` counts 100Hz jiffies: one tick in a full second is
   exactly 1.0%, and short/fractional buckets can read 1-2% from a
   single tick, so max1s cannot be the sustained <1% gate.

## Verification (release, gl/llvmpipe, 1200x800 Xvfb)
- `e2e-idle-cpu.sh` PASS over repeated complete runs:
  - S1 idle 0 frames / 0.0%; wake roundtrip 13-20ms.
  - S2 60Hz spinner: 16.9-18.2fps, process avg 16.1-21.1% (recorded
    max1s bursts up to 122.5%, which is why only avg is asserted).
  - S13 40k-line cat: 0.8-3.2% process average, fps <= 65.
  - S14 10Hz steady state: main avg 0.4-0.6%, main max1s 1.0-1.435%,
    process avg 5.6-8.0%, fps 1.0.
  - S15 8-pane idle: 0 frames, process avg 0.0-0.125%.
- `cargo test --workspace` green; `cargo clippy --workspace
  --all-targets -- -D warnings` green; `cargo fmt --check` green.
- P2 (vendored egui-wgpu + damage-region redraw) is NOT part of this
  change; the next lever remains full-surface raster/present on software
  adapters.
