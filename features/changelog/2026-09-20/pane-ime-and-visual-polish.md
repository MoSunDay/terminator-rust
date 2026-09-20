# Pane IME (input method) + visual polish round 2

Date: 2026-09-20 · Status: verified (unit + 7 e2e suites + real-XIM e2e x2)

## IME: 中文/日文 direct input into panes

Problem: with a pane focused there is no egui TextEdit, so egui-winit
never wrote `PlatformOutput::ime` -> `set_ime_allowed(false)` forever ->
XIM never engaged. 中文用户完全无法在 pane 里打字。

Facts uncovered (egui 0.36.1 / egui-winit 0.36.1 / winit 0.30.13):
- winit X11 IME is COMPLETE (full XIM client: Preedit/Commit via
  XFilterEvent, spot location). Requires an XIM server: XMODIFIERS=@im=
  ibus|fcitx. Bare Xvfb has none -> no Ime events at all there.
- egui-winit allows IME iff `PlatformOutput::ime` is Some (checked
  EVERY frame, per viewport); nobody validates it - a pane can write it
  directly, no TextEdit needed. X11 uses rect.min only as the XIM spot.

Change:
- `WindowUi.ime{,_cursor,_pane,_last_pane}`: per-window preedit text +
  the focused pane's cursor-cell anchor (screen.rs refreshes via the
  new `grid::cursor_rect`; `render/preedit.rs` paints preedit text +
  2px accent underline there).
- `input/ime.rs`: pure decision fn -> `IMEOutput{purpose: Terminal,
  interrupt-on-pane-change}` synced at the end of every windows::render
  (rename editors keep owning IME: the egui_wants_keyboard_input early
  return now also clears a stale pane preedit).
- `keyboard.rs`: `Event::Ime(Preedit)` latches (empty text = composition
  ended, the X11 signal), `Commit` delivers RAW UTF-8 bytes to the
  focused pane (`vtask::write`) - not the key encoder (CJK would hit
  Unidentified+utf8), not bracketed paste (IME commit is typed input).
  XFilterEvent already swallows composing keys -> no double input.
- `scripts/bin/e2e-ime.sh`: REAL XIM e2e - Xvfb + private/system dbus +
  `ibus-daemon --xim` + engine `libpinyin` + LANG=zh_CN.UTF-8: M1 typed
  'hanzi' never reaches the pty while composing, M2 preedit ink (purple
  -hue detector vs baseline: the 2px underline antialiases, exact
  accent match finds ZERO pixels; 0-ink = WARN+SKIP, PreeditNothing
  servers are legal), M3 space commits 汉字 + bare Shift_L toggles
  libpinyin EN for ASCII passthrough, M4 quit. CI: e2e-ime job (apt:
  ibus ibus-libpinyin dbus x11-utils locales; locale-gen zh_CN.UTF-8 -
  XIM locale negotiation needs it).

## Visual polish (好丑 round 2)

- Chip row: CHIP_H 24->28, insets 3->4 (nominal 36, RENDERED ~41: egui
  adds trailing item_spacing.y + a 1px panel offset - the old "30" was
  passing on tolerance), chip font 12.
- Focused pane: 1.5px accent ring + 2px rounded left bar + header tint
  mix(bg,accent,0.10) (`colors::focus_header`, unit-pinned); unfocused
  keeps chrome_bg.
- Cursor: 2px corners, peak blink alpha 0.9. Window: 1px hairline
  definition border (end of windows::render; edge-sampling audited).
- e2e-ui-style/dragdrop bands recomputed from OBSERVED geometry (G band
  44..62, H start X+260 clears the trailing '+' glyph span, I2 expects
  the focused tint); all 7 suites re-run green, 246 unit tests.
