# Settings text one notch smaller; opacity floor opened to 0.1

Date: 2026-09-22 · Status: verified (fmt/clippy(-D warnings)/
cargo test --workspace 302 passed green; e2e-ui-style.sh ALL GREEN
twice, incl. phase J font-scale)

## Problem

The Settings window - and the egui-styled floating chrome sharing its
text styles (context menus, tooltips, rename TextEdits) - rode on egui's
default sizes (Body 13pt), visually louder than the 12pt chrome labels.
The Window opacity slider floor 0.5 blocked deeper translucency.

## Solution

- ui/style.rs: text-style bases one notch below egui defaults
  (Body/Button/Monospace 11.5, Small 8.5, Heading 15), still scaled by
  chrome::scale(font_size); chrome labels use explicit FontId and are
  untouched, so every ui-style pixel gate stays intact.
- Opacity floor 0.5 -> 0.1: slider range (ui/inspector.rs) + load clamp
  (persist/settings.rs); the clamp unit test now probes 0.0, since 0.1
  is in range.

## Notes

- Ink/cursor/selection stay opaque below either knob (with_opacity /
  pane_bg_alpha policy unchanged).
- agents.md memory token synced (opacity 0.1..=1.0).
