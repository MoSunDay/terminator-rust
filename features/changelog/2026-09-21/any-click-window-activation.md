# Any-click window activation + gated chrome StartDrag

## Problem
On the deploy target (xfwm4, scale 1.5) clicking inside a secondary
window's PANE activated it, but clicking the tab strip ("window name"
chrome) or a tab chip did not - and a chrome drag afterwards moved
(0,0). Two stacked causes:

1. WM focus policy alone decides activation for unfocused-window
   clicks; chrome/chip presses lost the race (a pending move-grab can
   eat the click entirely).
2. `drag_started_by -> StartDrag` fired for press+motion COALESCED into
   one frame (target renders slow frames): a 3px drift-click sent
   _NET_WM_MOVERESIZE, whose pointer grab swallows the activation AND
   wedges the WM gesture - the next real drag was dead, and repeated
   StartDrag spam (pointer state flaps across viewport passes during
   the grab; global press_origin reads are cross-viewport contaminated
   - bogus 288px travel) kept re-thrashing openbox/xfwm4.

## Fix (crates/app)
- windows.rs: every viewport pass with `focused != Some(true)` and
  `pointer.any_pressed()` sends `ViewportCommand::Focus` (winit
  `_NET_ACTIVE_WINDOW`): ANY click - pane, chip, bare chrome -
  explicitly activates the window.
- ui::arm_window_drag (ui/mod.rs; used by tabs.rs chrome row +
  pane_header.rs lone-pane branch): StartDrag only after
  WINDOW_DRAG_MIN_PX = 8pt of RESPONSE-LOCAL accumulated
  `drag_delta` travel (never global press_origin), latched per
  button-down (`any_down`), reset on release OR fresh press
  (`any_pressed`) so a release swallowed by the WM grab cannot
  suppress the next gesture.

## Verified
- Local openbox + xfwm4: still/jitter(3px coalesced)/chip/pane clicks
  activate; consecutive chrome drags x3 each move the window with
  exactly one STARTDRAG per gesture; chip clicks after drags live.
- e2e: windows, window-controls, mouse-key, dragdrop all green.
- Deploy target 192.168.31.196 (xfwm4): STILL/JITTER-CHROME/
  JITTER-CHIP/CHIP-AFTER-DRAG/PANE all ACTIVATED, REAL-DRAG moves
  (150,75), 3 consecutive drags all move + pane click still activates.
