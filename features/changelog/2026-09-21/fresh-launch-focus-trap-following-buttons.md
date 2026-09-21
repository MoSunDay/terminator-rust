# Fresh launch, Tab focus trap, following chrome buttons

Date: 2026-09-21 · Status: verified (e2e-window-controls R1-R5+S1-S4,
mouse-key T1-T4, empty-restore E1-E3 green; fmt/clippy/tests clean)

## 1. Launch default is a fresh window
Opening the app used to silently restore the previous session. Now
main.rs launch_state + state.rs fresh_keep_prefs start a FRESH window
(one new tab) carrying only theme_name/settings across from state.json.
Session restore (PWindow/tabs) is opt-in via TERMINATOR_RESTORE=1 -
every e2e that presets state.json exports it.

## 2. The Tab key killed panes
egui 0.36: Sense::click()/drag()/click_and_drag() all carry the
FOCUSABLE bit. A bare Tab moved keyboard focus to the first focusable
widget of the frame - the pane-header X, a builtin egui Button. From
then on keyboard.rs's egui_wants_keyboard_input early-return swallowed
every pane key, and Space FIRED the focused button: do_close_pane on
the only pane quit the whole app (observed as "Tab then stuff closes /
hangs"). All app-owned interacts (chrome drag, tab chips + close,
tabs_widgets button cells, pane header + its X Button via
.sense(Sense::CLICK), dividers, pane body, edge strips) now use the
non-focusable Sense::CLICK / Sense::DRAG / Sense::CLICK|Sense::DRAG.
Only TextEdits (rename editors) stay focusable - IME needs it.

## 3. '+/split' group follows the chips
The zoom/split cells used to sit at a fixed right-edge slot even with
one short tab. tabs.rs GROUP_W/GROUP_PARK: group_left = last chip right
+ 8, vertically centered on the row, clamped so on overflow the group
parks left of the fixed edge cells ('+' center stays W-137, matching
the old fixed slot); CHROME_RESERVE=153 remains the chip-strip bound.
e2e-window-controls S4: preset 2 tabs, click chip-adjacent X+233
creates a 3rd tab (active=2), the old fixed X+W-137 must NOT.
