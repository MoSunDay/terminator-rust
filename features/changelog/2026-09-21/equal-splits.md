# Splits always divide the space equally

## Problem
Splits used the global persisted `settings.split_ratio` ("new pane
share" slider, default 0.5). Once the slider was dragged - the deploy
target had 0.46 - every future split (Ctrl+Shift+E/O, chrome split
buttons, drag-drop pane moves, cross-tab migration) divided
unevenly, with no visible reason why.

## Fix (crates/app)
- All split paths pin ratio 0.5; per-split tuning is the divider drag.
- `Settings.split_ratio` / `PSettings.split_ratio` / the Settings-panel
  slider are removed. Old state.json files still load: serde ignores
  the stale key and the next save drops it.
- layout-tree unchanged (`split_pane_ratio` stays for divider-drag
  persistence).

## Verified
- app 110 tests + layout-tree 56 green; e2e mouse-key + dragdrop green.
- Deploy target: Ctrl+Shift+E then Ctrl+Shift+O -> panes 63x36 /
  63x17 / 63x17; persisted tree `Split v 0.5` / inner `Split h 0.5`;
  stale 0.46 gone from state.json.
