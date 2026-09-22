# App logo + window icon ("split T")

## Problem
The app shipped with no icon: every OS window (root + secondaries)
showed egui's built-in default (the white "e" egg), on the desktop, in
taskbars/pagers and in the XDG autostart entry the deploy script writes
(no `Icon=` line at all).

## Design
- Glyph: a "split T" - purple `#bd93f9` top bar = the Chrome-style tab
  strip, purple stem, purple foot crossbar = a 50/50 split, and two legs
  in pink `#ff79c6` / cyan `#8be9fd` = the two resulting panes - on a
  rounded dracula-gradient tile. All colors come from the dracula preset
  in `crates/theme/src/builtin.rs` (background `#282a36`, selection
  `#44475a`, purple/pink/cyan ANSI slots), so the logo is the app's own
  default palette, not new artwork colors.

## Fix
- Generator `scripts/bin/gen-logo.py` (<=400 lines, pure Python +
  Pillow, no classes per the repo rules): 1024u design grid, rasters
  rendered on a 4x-supersampled master and LANCZOS-downscaled, sizes
  <=32px re-render with a bolder stroke; output is byte-deterministic
  (no timestamps/randomness), `--check` regenerates into a temp dir and
  fails on any byte drift.
- Asset set `assets/logo/`: `terminator-rust.svg` (vector master),
  `icon-{16,24,32,48,64,128,256,512,1024}.png`, `terminator-rust.ico`
  (multi-size), `terminator-rust.icns` (hand-built Apple ICNS).
- Runtime wiring `crates/app/src/app_icon.rs`: `include_bytes!` of
  `assets/logo/icon-256.png` -> `eframe::icon_data::from_png_bytes` ->
  `ViewportBuilder::with_icon` on the root viewport (main.rs) AND on
  every secondary OS window (`windows.rs::builder_for`), so all windows
  carry the same icon. Decode failure logs a warning and leaves the
  default icon (the app always starts).
- CI drift gate: `build-test` job in `.github/workflows/ci.yml` gained a
  "Logo assets up to date" step (`pip install pillow` +
  `python3 scripts/bin/gen-logo.py --check`) next to Format/Clippy;
  macos and e2e jobs untouched.
- Deploy: `scripts/bin/deploy-remote.sh` stage_build packs a hicolor
  tree (`share/icons/hicolor/{16,32,48,128,256}x{N}/apps/
  terminator-rust.png`, install -m 0644, inside the existing
  deterministic tar), and stage_autostart writes
  `Icon=$root/current/share/icons/hicolor/256x256/apps/terminator-rust.png`
  into the generated desktop entry (grep -Fx asserts Exec= and Icon=).

## Verified
- `python3 scripts/bin/gen-logo.py --check`: the committed assets are
  byte-identical to a fresh regeneration (Pillow 12.3.0).
- `cargo fmt --all -- --check` and `cargo clippy --workspace
  --all-targets -- -D warnings` clean; `cargo test --workspace` green,
  incl. the 4 new `app_icon` tests that pin the embedded 256px raster to
  the approved design (size, transparent tile corner, purple bar, pink /
  cyan legs) and prove every viewport shares ONE decoded icon.
- Live headless (Xvfb, scratch HOME/XDG_RUNTIME_DIR): the app logs
  `window icon: 256x256 rgba` + `winit::Icon::from_rgba; width=256
  height=256`, and `_NET_WM_ICON` is present on the ROOT window and on a
  Ctrl+Shift+N SECONDARY (2 X windows by WM_CLASS): CARDINAL/32 with
  65538 items (256*256+2), word (0,0)=0x0 (transparent tile corner),
  (128,66)=#bd93f9 (bar/stem), (83,170)=#ff79c6, (173,170)=#8be9fd -
  the logo, on both windows.
- `scripts/bin/e2e-windows.sh` PASS (W1-W6) - the secondary-window path
  that now also attaches the icon is unchanged behaviourally.
- GOTCHA: `xprop -id WID _NET_WM_ICON` prints the property NAME with an
  EMPTY value list here, which reads as "no icon"; the data is really
  there. Read it with XGetWindowProperty (AnyPropertyType) - and note
  format-32 items come back as `long` (8 bytes) on LP64, not 4.
- `bash -n scripts/bin/deploy-remote.sh` clean; ci.yml parses as YAML.
  The deploy itself was NOT run (live remote target), so the hicolor
  tree + desktop `Icon=` are staged-but-unexercised.
