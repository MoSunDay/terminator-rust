# App-driven edge resize with pointer polling

Date: 2026-09-20 · Status: verified (e2e-window-controls.sh R1a/R1b/R1c green under openbox-in-Xvfb; gates clean)

## The bug

`ViewportCommand::BeginResize` (EWMH _NET_WM_MOVERESIZE) gives the WM a
pointer grab that also swallows the ButtonRelease: egui keeps
any_down/potential_drag wedged (next gesture dead) and a WM that misses
the release keeps resizing after mouseup ("window follows the pointer").

## The fix: we own the gesture

input/resize.rs keeps a live `Gesture {dir, pointer, rect}` in
WindowUi.edge, re-derived EVERY frame from the press anchor
(press_origin, not latest_pos: the press frame can coalesce with the
first motion and bias it), sending InnerSize (+ OuterPosition on W/N
edges) as plain ViewportCommands; `resized_rect` is a pure fn that pins
the opposite side and clamps MIN_SIZE (400,300) on the grabbed side.
Events alone cannot drive W/N drags: openbox reconfigures on our
per-frame XMoveWindow and breaks the core grab. New pointer_poll.rs
(x11-dl) XQueryPointer's per frame while live - global
pointer + BUTTON MASK, release truth needs no event; poll None ->
event path stands in.

## e2e

R1a asserts the EXACT landed width (1200 -> 1280), R1b freezes geometry
after release against pointer wander, R1c drives the west edge (origin
100 -> 40); strip ids salted by window id (egui interaction state is global).
