Commit: 832f94ebca7792accf3a918f29d42cc130fb075b

# Verified end-to-end (final state)

Live-checked capabilities of the current build. Maintenance: keep
this list to what was actually verified - suite names/ids move here
only when the matching script exists and passed.

All suites green (ui-style, mouse-key, windows W1-W10, window-controls
R1-R6/S1-S5, ipc-oc, cjk, oc-exit, ime M1-M7, migrate M1-M5, remote);
per-script coverage in [agents/e2e-suites.md](e2e-suites.md); Xvfb
scripts export TERMINATOR_OPAQUE=1; live-verified
on deploy target 192.168.31.196 (2 X windows, per-window typing
isolation, window close keeps the app, opacity 1.0 = mocha bg not
black, ctl list/capture; remote px differ only via wallpaper blend).
- rendering: catppuccin + ANSI 256 bg exact px, CJK cmap (Han/kana/
  hangul) + real Han ink px, always-on chrome row (one tab keeps the
  bar; ui-style I), pane-title centering (ui-style G), no window-title
  row (H)
- windows/lifecycle: Ctrl+Shift+N second OS window, last-pane close
  removes the window, root last-pane close with sibling alive respawns a
  tab, Ctrl+Shift+W on non-last pane keeps app alive (K4), Ctrl+Shift+Q
  quits from any window, WM close honored; the chrome close X is a
  one-click CONFIRM MODAL (Quit = ROOT Close quits every window; Esc, a
  re-click on X and Cancel dismiss; keys stay out of the pane while it is
  up); borderless chrome-drag window move = live-desktop check
  only (StartDrag needs an EWMH WM - bare Xvfb probes stay 0,0)
- input: key echo, ^C echo through the egui Copy-fold gate, Ctrl+Shift+E
  split, SGR press/release/motion + wheel byte-exact (less wheel =
  arrows x3), cross-pane drag RELEASE to the press-owner pane, drag-
  select + Ctrl+Shift+C == xclip readback, Shift+drag escape hatch,
  Shift+PageUp/End scrollback paging + snap-back-on-typing, Ctrl+C
  interrupts a foreground job (pty signal reset verified)
- IME (real ibus + libpinyin over XIM): composing swallows the keys and
  paints the preedit underline, space commits 汉字 into the pty, bare
  Shift_L switches to EN passthrough; double-click rename editors (tab
  chip AND pane header) compose CJK from the FIRST key and the pane's own
  IME survives the editor closing
- opencode panes: real opencoder Ctrl+D/Ctrl+C exits the idle prompt
  (status 0), Ctrl+Shift+W closes a live TUI pane
- control channel: ctl list/capture/send/instances/migrate over the live
  socket (PaneInfo window u64 serde-default 0, WIN column after ID -
  column-count parsers adapt), TERMINATOR_SOCK in pane env, oc link via
  /proc fd discovery, submit -> pending -> consume -> receipt by seq,
  honest --wait, SIGTERM-stale socket reclaim (perms 600 on bind/reclaim)
- remote zellij: bootstrap create+attach, reconnect, exit-42 degrade;
  gate = loading cleared + typed marker round-trip + session listed
  ("ZELLIJ" never renders - chrome-free config)
- persistence: state.json save/restore across restart

See [agents.md](../agents.md) for the crate map and the
hard-won facts behind these checks.
