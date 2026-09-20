# Native title bar, always-on tab row, unified terminal settings

Date: 2026-09-20 · Status: verified (all 9 e2e suites green on Xvfb;
fmt/clippy -D warnings + workspace tests clean)

## Native decorations restored

`.with_decorations(false)` is deleted from the root ViewportBuilder
(main.rs) and `windows::builder_for` - the WM title bar is back as the
primary drag/resize/close surface. `.with_transparent(true)` stays
(window opacity + pane glass need it). The chrome drag handle
(tabs.rs chrome_drag) and the pane-header strip remain as conveniences
beside the title bar; on bare Xvfb (no WM) the change is a visual no-op.

## The tab bar renders unconditionally

Chrome-style: chips, '+', split buttons and the zoom/settings cells
stay visible with a single tab. The old "single tab = no top panel,
content starts at y=0" rule is deleted (windows.rs render(): the
`egui::Panel::top("tab_bar")` call is unconditional).

## Terminal appearance is one global Settings block

- `Settings` gains `font_size` (was per-window WindowUi state, never
  persisted - now uniform across every window AND persisted),
  `transparency` (glass 0..1) and `bg_color: Option<Rgb>`, both applied
  to ALL panes.
- `PaneMeta` loses `bg_color`/`transparency`; `WindowUi` loses
  `font_size`/`trans_open`/`color_open`/`color_buf`.
- The pane header keeps ONLY the X close button - the T/C buttons and
  the transparency/color popups are deleted. Settings (incl. the new
  "Terminal background" section: glass slider + swatches/hex) lives in
  the renamed Settings panel; the per-window flag is still
  `WindowUi.inspector`.
- grid.rs DrawArgs takes a precomputed `bg: Color32` + `fill_alpha`;
  `colors::effective_bg(palette, Option<Rgb>)` is settings-driven.
  Glass math unchanged: `pane_bg_alpha(transparency, opacity)` on every
  pane bg + ANSI cell/selection bg.

## state.json backward compatibility

`PSettings` moved to persist/settings.rs with the new serde fields
(defaults + clamps). Old files load unchanged - the per-pane
bg_color/transparency keys are silently ignored (serde unknown keys).

## e2e

- ui-style: preset + checks A/C/G/I moved to the settings model; I1/I2
  now assert the bar STAYS with one tab (chip fill at (X+50,Y+15),
  underline Y+30..Y+31, header tint pushed down to Y+44..Y+64, content
  from ~Y+66).
- cjk: ink scan starts below the chrome (Y+46; the chip row owns
  Y..~Y+42 and the Han line starts ~Y+69).
- mouse-key: Xvfb display pick uses the random-free-display probe
  (latent $$-socket flake).
