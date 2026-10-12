Commit: 8ecb8ab47bc992a9522edf0394616e0007744e47

# Verified end-to-end (final state)

Live-checked capabilities of the current build. Maintenance: keep
this list to what was actually verified - suite names/ids move here
only when the matching script exists and passed.

Previously verified UI suites (ui-style, mouse-key, windows W1-W10, window-controls
R1-R6/S1-S5, ipc-oc, notice N1-N9, cjk, oc-exit, ime M1-M7, migrate M1-M5).
Current checks pass for cjk, mouse-key and the expanded IME suite on both
debug binaries and the packed Linux release, using libX11 1.8.12;
per-script coverage in [agents/e2e-suites.md](e2e-suites.md); Xvfb
scripts export TERMINATOR_OPAQUE=1; live-verified
on a graphical desktop (2 X windows, per-window typing
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
- notice bubble: `terminator-ctl notice` (NO args, any arg = usage error
  rc!=0; writes the canonical OSC 9 bytes to the pane's own /dev/tty,
  best-effort ALWAYS exit 0 - one transport, no env/keeper/socket
  routing) marks the chip's blue dot rgb(70,150,255) in EVERY pane kind:
  plain local panes, the zero-binary `printf '\033]9;terminator-rust
  notice\007'` fallback, and panes behind a `terminator-session attach`
  (bytes traverse the keeper as ordinary PTY output, so replay re-fires
  the badge); repeats dedup to one dot; clicking the badged chip activates
  the tab and the focused window's ack clears it; an inactive tab keeps
  its dot while the focused window's ACTIVE tab notice stays suppressed
  - ALSO verified live on the real desktop instance (old deployed build, two app instances): IPC `notice <pane>` round-trips rc=0 over the per-instance socket and the user sees the chip dot on switching tabs (a focused ACTIVE tab consumes it instantly - by design)
- IME (real ibus + libpinyin over XIM): composing swallows the keys and
  paints the preedit underline, space commits 汉字 into the pty, bare
  Shift_L switches to EN passthrough; double-click rename editors (tab
  chip AND pane header) compose CJK from the FIRST key and the pane's own
  IME survives the editor closing; composing over existing colored text hides
  the covered glyphs, and Escape restores them (M3b pixel assertions).
  Delaying committed 汉字 echo by 1.2s while composing ni preserves all 183
  committed glyph pixels (M3c). Native panel checks pass on ordinary rows,
  hidden cursor, screen bottom, wrapped input and 900x600 resize (M3d).
  Exact row-end and wide-character wrap preserve all 180 committed glyph
  pixels; last-row preview preserves 182, Escape restores the original
  image, and actual commit follows the previewed wrap (M3e).
  macOS native panel and Fcitx runtime behavior remain unverified.
- release installer: 8 isolated tests cover immutable versions, corruption,
  stale retries, crash-before/after-switch, rollback, and rejection before
  remote upload/install when candidate verification fails. A packed release
  launched in private Xvfb before and after restoring a legacy install path;
  `/proc/PID/exe`, IPC and PTY round-trips matched the selected artifact.
  This does not prove a production deployment or a previous build’s behavior.
- opencode panes: real opencoder Ctrl+D/Ctrl+C exits the idle prompt
  (status 0), Ctrl+Shift+W closes a live TUI pane
- control channel: ctl list/capture/send/instances/migrate over the live
  socket (PaneInfo window u64 serde-default 0, WIN column after ID -
  column-count parsers adapt), TERMINATOR_SOCK in pane env, oc link via
  /proc fd discovery, submit -> pending -> consume -> receipt by seq,
  honest --wait, SIGTERM-stale socket reclaim (perms 600 on bind/reclaim)
- remote keeper: isolated tests verify named attach, retained shell state,
  concurrent clients, listing, exit-42 degrade and no Zellij invocation.
  Current keeper-based SSH interruption has not been live verified.
- reactive render loop (2026-10-12): idle 5s CPU 190 -> 22 ticks
  (~38% -> ~4.4% of a core, cursor still blinking) measured via
  /proc/<pid>/stat on the old vs new build; `terminator-ctl send` ->
  `capture` marker round-trip 52ms from a fully idle loop (PTY reader
  + IPC accept wakes both reach the ROOT viewport). Gate:
  scripts/bin/e2e-idle-cpu.sh. Same session re-verified: e2e-notice,
  e2e-ui-style, e2e-oc-exit, e2e-ipc-oc, e2e-mouse-key, e2e-cjk and
  e2e-bg-seams K1-K5 all green (bg-seams re-pinned to the dracula
  palette the 2026-09-27 sync installs - its old ghostty-default
  color pins had drifted stale, failing K1/K4 on BOTH builds).
- persistence: state.json save/restore across restart

See [agents.md](../agents.md) for the crate map and the
hard-won facts behind these checks.
