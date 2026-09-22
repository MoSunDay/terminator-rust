# Chrome text renders at the terminal font size

Date: 2026-09-22 · Status: verified (fmt/clippy(-D warnings)/
cargo test --workspace all green; e2e ui-style / window-controls /
dragdrop / cjk ALL GREEN post-commit)

## Problem

Tab chip titles rendered at 0.8x the terminal font (12pt at the 15pt
default) and egui-styled chrome text (Settings panel, context menus,
tooltips, rename TextEdits) sat at a fixed notch below the grid - the
chrome never matched the terminal it framed, and drifted at non-default
font sizes.

## Solution

- ui/chrome.rs: chip_font = font_size exactly (was 12.0 * scale);
  unit test pins 15 / 20 / 9.
- ui/style.rs: every TextStyle except Small maps to font_size exactly
  (Small keeps 8.5/15 * font for secondary hints); style reinstall
  stays memo-keyed on (theme, font) via UiState.styled_font.
- e2e probe geometry re-measured from real screenshots: ui-style F/I2
  chip-fill sample above the ink at (X+50, Y+7) - the 15pt "style [2]"
  label ends ~X+89 and close X starts ~X+96, too tight to sample;
  H bare-chrome scan starts X+270; window-controls S4 '+' click
  X+269 (chip width 106 -> 124px at 15pt).

## Notes

- Supersedes the earlier "one notch below egui defaults" bases from
  settings-text-scale-and-opacity-floor.md.
- window-controls S4 is a behavioral gate: the real '+' click at the
  new coordinate must spawn + activate a 13th tab.
- agents.md synced (chrome scales with the terminal font - chip_font
  EQUALS font_size, not base*scale).
