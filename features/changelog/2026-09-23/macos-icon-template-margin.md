# macOS icon: Apple icon-template margin (824/1024 body)

Follow-up to the logo asset set (2026-09-22 `app-logo-and-window-icon.md`).

## Problem
On macOS the app icon rendered a notch LARGER than every other app's, in
the Dock and in Finder. Our artwork filled 100% of the canvas, while
Apple's icon template keeps the body at 824/1024 of it (a 100u
transparent margin per side on the 1024 grid) - and AppKit scales the
WHOLE raster into the icon slot, so a full-bleed tile came out ~24%
bigger (1024/824) than a template-conformant neighbour. Two paths
publish our artwork verbatim on macOS:
1. `assets/logo/terminator-rust.icns` - the Finder/bundle icon
   (`pack-release.sh`'s darwin branch installs it into `Resources/`).
2. The runtime Dock icon: eframe's `AppTitleIconSetter` (eframe-0.36.1
   `src/native/app_icon.rs`) builds an `NSImage` from our RGBA and calls
   `NSApp setApplicationIconImage:`. winit's own `set_window_icon` is a
   NO-OP on macOS (`src/platform_impl/macos/window_delegate.rs`), so the
   raster `crates/app/src/app_icon.rs` embeds IS what the Dock shows.

## Design
- `MAC_BODY = 824`, `mac_margin(size) = size * (S - MAC_BODY) // (2 * S)`
  -> 1024: 100 margin/824 body, 256: 25/206, 32: 3/26. `mac_raster`
  LANCZOS-scales the master into the body and `alpha_composite`s it
  centered on a transparent canvas.
- NO shape change: `RADIUS/S == 0.225` already equals Apple's body
  corner ratio, so the tile rounds identically inside the body - only
  the margin was added.
- Per-platform split: macOS padded, everything else FULL-BLEED
  (Linux/Windows scale the icon into their own slot, so a margin there
  would render visibly smaller). `icon-{16..1024}.png`,
  `terminator-rust.ico` and `.svg` stay byte-identical; the new mac
  raster plus the icns (205356 -> 165588 bytes) are the only deltas.

## Fix
- `scripts/bin/gen-logo.py`: `MAC_BODY` / `MAC_SIZES` / `mac_margin` /
  `mac_raster`; `build()` writes the NEW `assets/logo/icon-256-mac.png`
  and builds the icns from the padded rasters; `parse_icns` now returns
  `[(ostype, payload), ...]` so `verify()` can open every entry;
  `verify_mac()` (transparent margin ring, inked body edge, `SAMPLES`
  hues remapped into the body) runs on `icon-256-mac.png` AND all six
  icns entries; a FULL-BLEED regression guard asserts edge alpha > 128
  on `icon-{48,256,1024}.png` (measured 239/195/254 - LANCZOS border
  clipping; a mac template reads 0 there).
- `crates/app/src/app_icon.rs`: `ICON_PNG` is now a cfg pair -
  `icon-256-mac.png` on `target_os = "macos"`, full-bleed `icon-256.png`
  elsewhere. Tests went geometry-derived: `ink_box()` (alpha bbox) +
  `design_px()` (maps the generator's 1024u design grid into that box),
  so ONE assert body is correct on both platforms (exact RGBA samples
  unchanged), plus two platform guards:
  `macos_icon_carries_the_apple_template_margin` -> `ink_box == (25, 25,
  206)`, `full_bleed_icon_reaches_the_canvas_edge` -> `(0, 0, 256)`.
- `pack-release.sh` / `deploy-remote.sh` needed no change: the darwin
  branch already installs only the `.icns`, and the Linux hicolor tree
  still points at the full-bleed `icon-*.png`.

## Verified
- `python3 scripts/bin/gen-logo.py` (13 files), then `--check` TWICE -
  byte-deterministic - plus `--preview`.
- `cargo fmt --all -- --check`; `cargo test -p app` 151 passed
  (`app_icon` 5); `cargo clippy -p app --all-targets -- -D warnings`.
- aarch64-apple-darwin cross `cargo check -p app --all-targets`: proves
  the `target_os = "macos"` include path and its cfg'd guard compile.
