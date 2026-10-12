# XIM parked-panel rescue: candidates follow the composing row on libX11 1.7.x

## Problem
`e2e-ime.sh` M2 failed on this box (libX11 1.7.5) for BOTH the current
and the Oct-7 build: the ibus candidate panel sat docked at the
window's bottom-left instead of following the insertion row, while the
app's own preedit underline painted correctly. The Oct-7 "ime M1-M7"
verification ran on a libX11 1.8.12 machine - the gate had never been
green here.

Root cause (not app logic): winit negotiates the XIM preedit-callback
style, and under that style libX11 IGNORES `XNSpotLocation`
(freedesktop bug 1580; winit's own `x11/ime/context.rs::set_spot`
documents it: "certain IMEs do show selection window, but it's placed
in bottom left of the window and couldn't be changed"). A same-env
xterm reference (no callbacks) had a perfectly following panel,
proving the IM stack fine.

## Change Summary
- `crates/app/src/input/ime/popup.rs`: new pure `follow_row(caret,
  panel, monitor)` + `NEAR_ROW_PX = 24`: a panel farther than 24px from
  the caret row gets a target just below it (`caret.bottom() + 1px`
  seam so it never covers the row; above the row when the monitor has
  no room below; None when nothing fits). A panel already near the row
  keeps its native placement.
- `crates/app/src/input/ime/x11.rs`: `repair(..., composing)` now also
  applies `follow_row` while composing (overlap repair unchanged and
  still first); the monitor lookup is factored into `monitor_of`.

## Verification
- 2 new popup unit tests (seam, near-band stability, flip-above,
  no-room).
- `e2e-ime.sh` ALL GREEN (M1-M7, native panel + delayed echo +
  wrap/cancel) on this libX11 1.7.5 box.
- Full workspace gates re-run: 414 tests, clippy 0, fmt clean.
