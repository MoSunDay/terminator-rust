# Borderless again: the tab strip is the drag surface

Date: 2026-09-20 · Status: verified live (openbox-in-Xvfb + deploy
target xfwm4); all 9 Xvfb e2e suites re-ran green on the borderless
build; fmt/clippy/tests clean

## Revert of the native title bar (~1h experiment)

`.with_decorations(false)` is back on the root ViewportBuilder (main.rs)
and `windows::builder_for`; `.with_transparent(true)` stays (window
opacity + pane glass need it). User preference: the tab strip IS the
title bar - the always-on Chrome-style row (chips, '+', split,
zoom/settings, kept from the previous commit) owns that vertical space.
Close is Ctrl+Shift+Q / WM close / last-pane close; no decorations to
click.

## Dragging stays on the cross-platform StartDrag path

Window move = bare-chrome drag on the tab row (tabs.rs chrome_drag ->
`ViewportCommand::StartDrag` -> winit `drag_window()`), and a lone
pane's header keeps the same role (pane_header.rs). This is
cross-platform by construction (macOS performWindowDrag / X11
_NET_WM_MOVERESIZE via the WM / Wayland toplevel move / Windows) -
deliberately NO platform-specific drag code; macOS support is untested
(the app builds/tests on Linux only). Verified live: the window moves
by the exact drag delta (0,35 -> 160,105 for a +160/+70 drag on the
deployed borderless build under xfwm4; same under openbox-in-Xvfb).

## Gotcha: xdotool search --name matches substrings

On the deploy target the user's PYTHON terminator emulator (WM_CLASS
"terminator","Terminator") has the tab title
"root@ds: ~/terminator-rust" - `xdotool search --name terminator-rust`
returns BOTH, and driving the wrong window made StartDrag look broken.
Target the app by WM_CLASS ("", "terminator-rust") or a known WID,
never by name alone.

## e2e consequence

Chrome-drag window move CANNOT be asserted on bare Xvfb (no WM ->
_NET_WM_MOVERESIZE goes nowhere; probes show 0,0 -> 0,0) - it is a
live-desktop check only. Decorations were a geometry no-op on Xvfb
either way; all 9 suites re-ran green.
