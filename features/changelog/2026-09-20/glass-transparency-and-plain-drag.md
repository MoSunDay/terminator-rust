# Pane transparency is real glass + plain-drag pane rearrangement

Date: 2026-09-20 · Status: verified (unit pins + e2e-dragdrop D1-D5 +
e2e-ui-style ALL GREEN + redeploy smoke)

## Glass: the per-pane 0..1 slider now means see-through

User feedback: the header popup slider labeled "pane bg" only blended
the pane color into the theme background - turning it up never showed
the desktop ("透明是什么意思？玻璃"). Semantics change:

- `PaneMeta.transparency`: 0 = opaque, 1 = fully see-through glass.
- `render::colors::pane_bg_alpha(t, opacity) = (1 - t) * opacity` is
  the fill alpha for the pane bg AND explicit ANSI cell / selection
  backgrounds (uniform glass; ink, cursor and selection text stay
  opaque). Window opacity still multiplies on top.
- `theme::blend_background(p, color)` drops the t param: color-only
  (user tint or theme bg); alpha is the renderer's job.
- Cursor-block glyph ink uses the bg RGB at full alpha (`bg_ink`) -
  the semi-transparent fill would render the on-cursor glyph invisible.
- Popup: "0 = opaque, 1 = glass (see through to the desktop)"; the
  stale "set a pane bg color first" warning is gone (glass works
  without a pane tint).
- Fails safe: non-finite t/opacity clamp to opaque; e2e presets run
  t=0, so pixel asserts stay byte-identical (ui-style ALL GREEN).

## Plain-drag header rearrangement

User feedback: dragging a pane title only moved the whole window;
rearranging panes (flipping a top/bottom split into left/right) was
hidden behind Ctrl+drag. Now `header_starts_pane_move(ctrl, panes)`:
a primary press on a pane header latches the pane move whenever the
tab has >= 2 panes (Ctrl still forces it); a lone pane keeps the
OS-window StartDrag. e2e-dragdrop D5 pins the bare gesture (same tree
result as the Ctrl version D1).

## Deploy

deploy-remote.sh stage_restart now probes the desktop user's IM daemon
and exports XMODIFIERS (@im=fcitx / @im=ibus) for the ssh-launched
instance - the desktop session env is not inherited over ssh, so pane
IME was dead on the deploy target.
