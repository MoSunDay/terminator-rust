Commit: a57e1b153dbd549e6ce240ffa798f0d17c2edc76

# agents.md - repo memory for terminator-rust

Pure-functional Rust (no classes) terminal multiplexer: egui 0.36 front-end
+ ghostty VT engine. One file <= 400 lines. No unwrap outside tests.

## Crate map
- layout-tree: tab/pane tree, splits, focus, `layout_tab` geometry
- vt-pane: PTY sessions; `spawn_session/pump/frame/resize/send_key/paste`; open_pty = posix_openpt+O_CLOEXEC pair, pre-fork argv/env/PATH tables (child branch is async-signal-safe only)
- theme: palettes + xterm 256 cube + `blend_background`
- remote: ssh -tt + zellij bootstrap (exit 42 = no zellij -> degrade)
- ipc-proto: serde wire types for the UDS control socket (Request/Response)
- ctl: `terminator-ctl` CLI: list/capture/send + `oc` link/submit/status/
  sessions; /proc discovery of the pane's opencoder process
- oc-store: direct rusqlite access to opencoder per-workdir stores
  (schema guard v18, insert/pending/receipts; `oc-store-fixture` dev bin)
- app: egui UI; binary `terminator-rust`; MULTI-OS-WINDOW: AppState
  {windows: Vec<WindowState{id,tree,WindowUi}>, active=rendering idx,
  focus=user window}; state at ~/.config/terminator-rust/state.json
  (PWindow[] + legacy tabs mirror); UDS ipc in src/ipc/;
  actions::close_exited auto-closes EXITED panes 250ms (EXIT_GRACE)
  after the exit was seen - exit 42 stays for auto_degrade

## Build/e2e
- `cargo build/test --workspace` (PKG_CONFIG_PATH set by .cargo/config.toml)
- headless UI smoke: Xvfb :NN + xdotool (type into window works; needs
  `xdotool windowfocus` - no WM focus otherwise)
- remote e2e: `cargo test -p remote --test zellij_e2e -- --ignored`
  (needs local sshd key auth + zellij; cleanup uses delete-all-sessions
  --force)
- control-channel e2e: `scripts/bin/e2e-ipc-oc.sh` (Xvfb + fake opencoder
  holding a fixture store open in a named pane; covers list/capture/send,
  oc link/submit/status/--wait, stale-socket reclaim after SIGTERM)
- mouse/key e2e: `scripts/bin/e2e-mouse-key.sh` (Xvfb + xdotool over the
  live socket: bare Ctrl+C ^C echo, cross-pane drag SGR press/motion/
  RELEASE landing in the press-owner pane, Shift+PageUp/End scrollback
  paging, Ctrl+C actually interrupting a foreground job)
- multi-window e2e: `scripts/bin/e2e-windows.sh` (Xvfb: Ctrl+Shift+N
  spawns a real second X window, cross-window typing isolation via ctl
  capture, last-pane close removes the window, re-spawn, quit-from-
  secondary kills the app, W6 root last-pane close with a sibling alive
  respawns a fresh root tab instead of quitting). CI runs it in
  .github/workflows/ci.yml (zig via PyPI wheel + fetch-vendor.sh for the
  ghostty pin; fmt/clippy are hard gates since that workflow landed);
  e2e-dragdrop.sh runs there too (needs scrot + python3-pil).
- drag-and-drop e2e: `scripts/bin/e2e-dragdrop.sh` (Xvfb + xdotool +
  scrot/PIL + state.json tree asserts; D1/D2 = Ctrl+drag pane header to
  sibling edge/center with mid-drag overlay pixel checks, D3 = chip
  reorder persisted, D4 = alive + quit).
- empty-window restore e2e: `scripts/bin/e2e-empty-restore.sh`
  (Xvfb + xdotool; E1 = `windows:[{tabs:[]}]` restores a live tab,
  E2 = exit auto-closes + app quits persisting empty tabs, E3 = the
  loop relaunches live; wired into ci.yml as e2e-empty-restore).
- IME e2e: `scripts/bin/e2e-ime.sh` (REAL XIM chain: Xvfb + dbus
  session bus (private fork fallback) + ibus-daemon --xim + engine
  libpinyin + LANG=zh_CN.UTF-8; M1 composing 'hanzi' never reaches the
  pty, M2 preedit ink purple-hue-detected vs a baseline scrot, M3 space
  commits 汉字 + bare Shift_L toggles libpinyin EN for ASCII, M4 quit;
  CI job e2e-ime apt: ibus ibus-libpinyin dbus x11-utils locales +
  locale-gen zh_CN.UTF-8 - XIM locale negotiation needs it).
- opencoder exit e2e: `scripts/bin/e2e-oc-exit.sh` (Xvfb + REAL
  /root/opencoder binary; OC_BIN override; SHELL wrapper that `exec`s the
  binary so pane pid == opencoder pid -> `kill -0` is exit ground truth; dummy
  ~/.opencoder/config.json needed - onboarding form eats ^C; focus navigation
  via Ctrl+Shift+Right, NEVER clicks into mouse-tracking panes; K4 =
  Ctrl+Shift+W on a NON-last pane keeps the app alive (quit only on the
  last close), K6 = a click closes a DEAD pane; opencode FIRST RUN seeds
  ~/.opencoder (skills installer + state dirs) - concurrent first-runs
  RACE it and instances exit(1) silently after "first frame" (repro: new
  HOME x3 pty = 2 dead, warm HOME = 3/3 alive), so the script WARMS UP the
  scratch HOME with one throwaway pty run before launching the app)
- cjk font e2e: `scripts/bin/e2e-cjk.sh` (fontTools cmap coverage of BOTH
  embedded subsets + a 2:1 advance probe on the Maple one; Xvfb live app: ctl
  send `echo 汉字测试` capture round-trip + scrot/PIL connected-ink
  assertions - real Han ink ~11-13x12-13px with interior strokes; ASCII
  <=8x12, wide filter 8.5..22 x 10.5..22, tofu square is hollow inside)
- ui style e2e: `scripts/bin/e2e-ui-style.sh` (Xvfb + scrot/PIL pixel
  assertions: pane bg/theme blend via transparency, gutter two-tone
  (chrome_bg field + 2px rounded grab handle, hover step mix 0.22 only
  under the pointer), chrome top-bar fill, active-chip underline (rounded
  caps, inset past the pill corners) + fill; expectations are COMPUTED
  in-script from dracula constants via a mix() helper - token retunes
  touch only colors.rs/tokens.rs, geometry changes touch the script;
  presets a dracula Split state.json;
  checks I1/I2 assert the single-tab zero-chrome layout)
- deploy: `scripts/bin/deploy-remote.sh` one-click (deterministic dist/
  repack, sha256 gate BOTH ends, /opt/terminator-rust/current symlink,
  XDG autostart for the desktop user, pid-kill restart, ctl smoke).
  Target 192.168.31.196: user m, DISPLAY=:0, XDG_RUNTIME_DIR=/run/user/1000.

## Hard-won facts (do not relearn)
- SIG_IGN survives fork+execve: an app started in the background (shell &,
  desktop launchers) leaves SIGHUP/INT/QUIT/TERM ignored in every pane
  child, and ^C/^\ then do nothing. vt-pane/src/pty.rs child branch resets
  HUP/INT/QUIT/TERM/PIPE to SIG_DFL; do not remove.
- ghostty ABI is exact-revision pinned: libghostty-rs rev 8272abe <->
  ghostty 22d1317 <-> vendored third_party/libghostty-vt (gitignored).
  Rebuild: scripts/fetch-vendor.sh (zig 0.16.0 via `pip install ziglang`,
  ziglang.org blocked, PyPI works).
- EVERY crate linking ghostty (even transitively) needs direct
  `libghostty-vt-sys = { workspace = true }` for pkg-config feature
  unification, else vendored-zig build kicks in.
- zellij >=0.39 config.kdl: theme colors must be one-per-line (KDL v1
  needs `;` between same-line siblings) AND all 16 ANSI slots required
  (black/white/bright_*); we derive them from the 9-slot palette
  (bright_*=base, black=bg, white=fg).
- ghostty vt resize takes 4 args (cols, rows, cell_w_px, cell_h_px);
  key::Encoder options via set_options_from_terminal (no Result).
- egui Key letters are `Key::A..Z`; ghostty key::Key letters are plain
  `A..Z`; app/input/keymap.rs maps them.
- ghostty legacy key encoder emits NOTHING for Alt+printable unless the
  KeyEvent utf8 is set (probe: ALT+B utf8=None -> []; utf8="b" -> ESC b);
  ctrl codes encode fine without text. egui-winit on X11 delivers Key AND
  Text for Alt+letter - drop the Text duplicate (alt_keyed_chars).
- Programs (zellij) stall on "Loading Zellij / Querying terminal emulator"
  until DA1/DA2/DA3 + XTWINOPS size + color-scheme queries are answered:
  vt-pane/src/effects.rs installs the callbacks; resize keeps cell_px live.
- ghostty mouse: `set_options_from_terminal` AND the `.size` setopt BOTH
  reset the encoder's `last_cell` (per-cell motion dedup) -> refresh
  options/size ONLY on change; vt-pane mouse.rs PointerState caches
  last_modes (mouse DEC-mode bitset: 9/1000/1002/1003 + 1005/1006/1015/1016)
  + last_size - resync on ANY mode change, not just tracking on/off. vt_write returns (), not Result.
- pane rect can exceed the whole-cell grid (746px vs 45x16=720px) ->
  clicks below the last row are OUT of grid and silently null a
  selection; ALWAYS clamp_grid_px (vt-pane/src/mouse.rs) before
  encode_mouse/select_*.
- wheel routing (bash/GNOME-Terminal habit baseline): tracking mode ->
  button 4/5 press-only per line; alt-screen -> arrows x3; else
  viewport scroll x3; Shift = local-selection escape hatch even while
  tracking. WHEEL_STEP_LINES=3; X11 wheel = Line +/-1 per notch.
- egui-winit folds Copy/Cut/Paste using its INTERNAL modifiers, but ctx
  i.modifiers is the POST-batch aggregate; a fast ctrl+c whose ctrl-down
  marks land in an earlier frame than the folded Event::Copy looks
  modifier-less -> ^C silently rerouted to the clipboard path (e2e T4
  "echo but no interrupt" shape). keyboard.rs seeds per-event mods from
  UiState.mods_frame_end (previous frame end, updated on every handle()
  incl. the text-field early return) then advances over ModifiersChanged
  marks; batch shapes like [COPY, Key(ControlLeft,false,ctrl), Mods(NONE)]
  are normal, do not "fix" them by trusting i.modifiers.
- copy path: egui-winit folds ctrl/cmd+C/X/V (shift variants and dedicated
  keys too) into Event::Copy/Cut/Paste and emits NO Key event; keyboard.rs
  gates on ctrl&&!shift: bare Ctrl+C/X/V forwards ^C(SIGINT)/^X/^V to the
  child, other forms -> actions::copy_focused (arboard, skip empty); follow_output(s) after keys/paste so typing snaps
  scrollback to live. context_menu suppressed while tracking.
- scrollback review (batch 2 slice): Shift+PageUp/PageDown pages the
  focused pane's local viewport, Shift+Home/End jump top/live (intercepted
  in app/input/scroll.rs BEFORE child pass-through; ctrl forms stay
  shortcuts). Focused pane draws a 2px viewport bar while unpinned
  (vt-pane/src/viewport.rs: page_delta = rows-1, one context line overlap).
- pointer grabs are per-button (uist.pointer_pane owner +
  pointer_buttons bitmask, pointer_last = lowest bit still held):
  releases follow the press owner across pane borders, chording holds
  the grab until the last release, and an owner pane vanishing mid-grab
  (tab switch / close) drops the whole grab so motion/wheel never target
  a zombie; buttonless 1003 hover motion stays unforwarded (batch-2
  gap). app surface_px multiplies egui points by pixels_per_point
  (vt-pane cell px are physical).
- Xvfb initial pointer rests at the SCREEN CENTER - with a centered
  50/50 split that is exactly ON the gutter, so any pointer-state visual
  (divider hover handle) is live from frame one: e2e-ui-style parks the
  pointer on bare chrome (`xdotool mousemove`) + sleeps ~0.5s before
  scrot (the app repaints on a 50ms cadence; an instant scrot grabs the
  pre-motion frame). All Xvfb e2e scripts export TERMINATOR_NO_MOTION=1
  (hover fades + cursor sine blink pinned to end states; see
  render/tokens.rs - radius/shadow/motion token layer, pure functions).
- Xvfb smoke ops: launch with `setsid nohup ... </dev/null &` to survive
  across tool calls; NEVER `pkill -f` a pattern that occurs in your own
  command line (self-kill, tool exit -1) - kill by PID instead.
  Ground truth: select text + Ctrl+Shift+C then `xclip -o -selection
  clipboard`; pixels via scrot + PIL. `xdotool --window <id> key/click`
  sends XSendEvent, which winit DROPS - focus the window
  (`xdotool windowfocus`) and use the global `xdotool key/click` (XTest)
  instead; bit us again driving the live app on the deploy target.
- libghostty default palette green ~(181,189,104) red ~(224,108,117) -
  color tests assert dominance, not VGA values.
- control socket: $XDG_RUNTIME_DIR/terminator-rust/ipc.sock (fallback
  ~/.config/terminator-rust/); start() live-probes and reclaims stale files
  (SIGTERM runs no destructors, the file survives; next start removes it);
  bind_private() chmods it 0600 fail-closed (capture/send is full remote
  control; the ~/.config fallback dir is traversable on common distros);
  pane children inherit TERMINATOR_SOCK; manual pane titles are unique
  addressing keys - pane rename rejects duplicates AND digits-only names
  (untagged PaneSelector would parse them as pane ids), refusal reason is
  shown in red under the editor; /proc db discovery errors listing all
  candidates when one opencoder process holds several distinct stores
  (never silently link the wrong workdir).
- oc submit writes session_inputs (delivery steer|queue) straight into the
  opencoder store (schema guard PRAGMA user_version=18, BEGIN IMMEDIATE,
  admitted_seq = MAX+1); the opencoder TUI is the SOLE runner - rows drain
  only at its turn boundaries (steer) / idle (queue); a TUI left idle
  strands pending rows until its next interaction (their idle_rekick hook
  exists but nothing polls it; --wait reports timeout honestly).
- vendored ghostty key encoder silently drops ALL letter keys
  (plain/shift/ctrl, utf8=None) whenever the terminal pushed ANY kitty
  keyboard flags (flags=1 alone kills Ctrl+D -> [] instead of [4]);
  Enter/C0-still-fine keys and Escape (CSI-u) encode, and utf8 text rescues
  plain/shift letters (send_char sets utf8) -> ONLY Ctrl+letter with utf8=None
  breaks. Fix in vt-pane/src/task.rs send_key: mods==CTRL + letter + flags
  non-empty -> set_kitty_flags(empty) on ENCODER OPTIONS ONLY (terminal real
  state untouched, next key's set_options_from_terminal restores). NEVER "fix"
  it by setting utf8 on ctrl combos - encoder then emits the letter byte
  instead of the C0 byte.
- real opencoder (source /root/opencoder, crossterm 0.28) pushes kitty flags 7
  (`\x1b[>7u`), NOT 11; accepts BOTH legacy bytes and kitty CSI-u for
  Ctrl+letter (raw pty: 0x03/0x04/`ESC[99;5u`/`ESC[100;5u` all exit
  cleanly). Without ~/.opencoder/config.json it sits in the onboarding form
  where Ctrl+C only cancels (only Esc/Ctrl+D exit); a dummy
  provider/base_url/api_key/model config passes local validation with no
  network and gives the idle prompt. GESTURE DRIFTS with the OC_BIN
  build: some builds exit the idle prompt on a SINGLE Ctrl+C or a bare
  Ctrl+D (status 0 both, Esc inert), others need Ctrl+C TWICE with
  Ctrl+D inert. e2e K2 uses spaced Ctrl+C x2 (valid under both); K3
  rides Ctrl+D-exits as POSITIVE proof of 0x04 delivery through the
  kitty-flags workaround (dropped key = pane stays alive). Probing
  gestures on a raw pty: write the MASTER side (pty.openpty +
  os.write) - writing /dev/pts/N (slave) simulates terminal OUTPUT,
  keys never reach the child, and every "inert" observed that way is
  void.
- IME (pane input method): winit X11 XIM is COMPLETE (XFilterEvent
  swallows composing keys; the ime rect's min is the XIM spot) but
  needs an XIM server (XMODIFIERS=@im=ibus|fcitx) - bare Xvfb has
  none, Ime events never fire there. egui-winit allows IME iff
  PlatformOutput.ime is Some EVERY frame (per viewport, nobody
  validates it - no TextEdit needed); app/input/ime.rs writes it
  (purpose Terminal) anchored at grid::cursor_rect; rename editors own
  IME via the egui_wants_keyboard_input early return, which also
  clears the pane preedit. Ime::Commit delivers RAW UTF-8 bytes
  (vtask::write) - never the key encoder (CJK lands Unidentified+utf8)
  nor bracketed paste (commit is typed input); Preedit with empty text
  = composition ended. e2e realities: the 2px accent preedit underline
  ANTIALIASES (exact-color match finds zero pixels - hue detector vs
  baseline scrot), and libpinyin starts in CN mode (all latin keys
  compose: probe shell liveness via ctl, ASCII needs the bare Shift_L
  EN toggle).
- chrome row height: nominal constants are NOT the rendered height -
  egui adds the row's trailing item_spacing.y plus a 1px panel offset
  (CHIP_H 28 + 2x4 insets = 36 nominal, ~41 rendered; the old 24+2x3
  was 30 nominal / ~35 rendered and passed on tolerance). e2e band
  literals were recomputed from OBSERVED pixels - trust scrot, not
  arithmetic.
- shortcuts: Ctrl+Shift+Q = global quit (Action::Quit -> ROOT viewport
  Close via send_viewport_cmd_to, so the press works from ANY window;
  to_skey must map Key::Q); Ctrl+Shift+N = new OS window
  (Action::NewWindow -> windows::spawn); pane title centering
  (pane_header.rs draws at title_rect.center() CENTER_CENTER) is
  pixel-asserted by e2e-ui-style.sh check G; the chrome is a SINGLE row -
  the old centered window-title row is deleted (it duplicated the tab name
  a third time under the chip and the pane header); zoom/inspector cells
  are anchored to the chip row's right edge via ui.interact on fixed rects
  and check H asserts title-text ABSENCE in the bare chrome zone; rename
  editors (pane + tab) cancel on outside click via `i.pointer.any_click()
  && interact_pos().is_none_or(...)` so a miss-click cannot strand the
  editor.

- drag-and-drop (2026-09): tab chips are Sense::click_and_drag() - a
  primary drag latches WindowUi.tab_drag (anchor = tab's lowest pane id,
  immune to index shifts; drag also selects the tab), the dragged slot
  stays an empty gap, a ghost chip (selected look) paints after the row,
  and layout_tree::move_tab runs live as the ghost center passes other
  chips' centers; active_tab FOLLOWS the moved tab to its new index.
  While latched, chrome_drag StartDrag is suppressed (no OS window move).
  Primary-drag on a pane HEADER latches WindowUi.pane_drag when the tab
  has >=2 panes OR Ctrl is held (header_starts_pane_move; a LONE pane
  keeps StartDrag = OS window move - nothing to rearrange); screen.rs
  suppresses divider
  interaction + raw pointer routing while latched, drop target = topmost
  pane whose FULL rect (header included) contains the pointer, zone via
  layout_tree::zone_for (center 50% square = Center/id-swap, else nearest
  edge = actions::do_move_pane detach+re-split at settings.split_ratio);
  overlay = render/dropzone.rs SOLID mix-ladder colors (no alpha) painted
  after dividers. e2e-dragdrop.sh covers the gestures (D1/D2 ctrl,
  D5 bare-drag) incl. mid-drag
  overlay pixels (D1 fill mix(bg,accent,0.22)) and state.json tree
  asserts; chips need >6px movement (egui click/drag disambiguation) so
  the e2e drags in steps, AND the e2e sleeps ~0.3s between mousedown and
  the first move: egui hit-tests per frame at pointer.latest_pos(), so a
  press coalesced with the first move latches the drag on the WRONG chip
  (or bare chrome -> StartDrag) - that race flaked D3 ~1/9 runs before
  the settle was added.

- font stack: ui/fonts.rs installs TWO embedded OFL fonts. PRIMARY
  assets/fonts/MapleMonoNF-CN-subset.ttf (7.4MB, 18780 glyphs, from maple-font
  v7.9 MapleMonoNormal-NF-CN-Regular; pyftsubset --no-layout-closure
  --no-hinting, unicodes = latin/symbol ranges + GB2312 hanzi enumerated via
  the codec itself (bytes([b1,b2]).decode('gb2312'), 7446 chars) + NF PUA
  E000-F8FF + planes 15/16 icon ranges) FIRST in both family chains
  (insert(0)) - ASCII, box-drawing 2500-257F complete, GB2312 Han, kana,
  NF icons; adv(汉) == 2*adv(M) exactly (1.2em vs 0.6em) so
  CellSize.wide_size converges to ~font_size (measured 13.99 at 14pt) -
  Han/latin mix now aligns with NO compensation scale. Maple lacks Hangul,
  fullwidth latin, ①㈱ etc -> they fall through to the LAST-chain
  assets/fonts/NotoSansSC-Regular-subset.otf (9.8MB, 40330 glyphs, recipe
  NEEDS --no-layout-closure; its Hangul advances 1.0em and paints slightly
  inside the 2-cell pair). ghostty advances NF PUA icons TWO cells (EAW
  ambiguous) so icon ink ~14-16px fills the double span. TERMINATOR_FONT and
  TERMINATOR_CJK_FONT=path[:ttc_index] swap each embedded font (last ':' +
  u32-suffix = face index); env fonts are parse-validated at startup (skrifa
  FontRef::from_index - epaint's own parser, direct workspace dep):
  unreadable OR unparseable -> log::warn + embedded fallback, never the
  epaint first-layout panic (app always starts). e2e ink detection: use
  (column-run x row-run) components and expect >=3 wide (w-delta >=8.5;
  ASCII tops out at 7) + >=3 stroked (a tofu square passes width but has a
  hollow interior).
- cell pitch quantization: epaint rasterizes UNHINTED glyphs at exact
  float positions - a fractional cell.w (Maple 0.6em = 8.4pt at font 14)
  cycles the subpixel phase per column (measured H deltas {8,9} stdev
  0.49 -> reads as fuzzy text + uneven letter spacing at ppp 1.0);
  grid.rs measure_cells snaps w/h to whole DEVICE pixels and derives the
  wide scale from the snapped w (wide cell == 2 narrow cells always).
  DEFAULT_FONT_SIZE 15 makes the Maple pitch naturally integer
  (0.6em*15 = 9.0px; 20/25 exact too; 14 snaps 8.4->8 with a 4.8% wide
  squeeze absorbed by wide_size). e2e-cjk ink filters tuned for 15pt
  (wide split 10.5/11.5, Han 12-15px). After: H deltas {9} stdev 0.
  Xvfb in scripts: NEVER derive the display number from $$ (stale
  /tmp/.X11-unix sockets make Xvfb refuse to bind) - probe random free
  numbers, and `export DISPLAY` for xdotool/scrot (env DISPLAY=... on the app
  line alone is not enough).
- window shell (borderless batch): eframe decorations off + transparent
  viewport (clear_color = TRANSPARENT); Settings.opacity (0.5..=1.0, default
  1.0 = opaque, persisted via serde default; legacy files load 1.0 too -
  transparency is OPT-IN because a compositor-less X renders transparent
  pixels black) multiplies alpha through
  render/colors.rs::with_opacity on chrome/pane base fills ONLY - Color32 is
  premultiplied, so text/cursor/selection/focus stroke stay opaque.
  Per-pane transparency (header popup slider) is REAL glass since
  2026-09-20: fill alpha = colors::pane_bg_alpha(meta.transparency,
  settings.opacity) (0 = opaque, 1 = see-through), applied uniformly to
  the pane bg AND ANSI cell/selection bgs; theme::blend_background is
  now color-only (tint or theme bg - the old blend-into-theme-bg
  semantics is GONE); cursor-block glyph ink reuses the bg rgb at FULL
  alpha or text goes invisible on glass. e2e presets run t=0 ->
  byte-identical pixels.
  TERMINATOR_OPAQUE=1 pins 1.0 at startup; every Xvfb e2e script exports it
  (no compositor -> no blending -> unstable pixels). With no titlebar the
  tab-bar background (tabs.rs chrome_drag) and the pane header strip are the
  drag handles: Sense::drag registered BEFORE the chips/buttons (topmost
  widget wins the click), StartDrag on primary press. Single tab = NO top
  panel (content starts at y=0; pane header identifies the pane; Inspector
  lives in the pane context menu); a second tab brings the chip row back.
- dead-pane corpses are gone: an exited session's pane auto-closes via
  actions::close_exited (EXIT_GRACE 250ms after the exit was SEEN) reusing
  do_close_pane semantics (root+only window quits, secondary removes
  itself, root-with-sibling respawns a tab). exit 42 is exempt
  (auto_degrade owns it); spawn-backoff panes (no session yet) are never
  corpses. The old e2e "click closes dead pane" is UNREACHABLE now
  (oc-exit K6 deleted, K2/K3 assert the auto-close instead).

- empty-window restore: state.json `windows:[{tabs:[]}]` (what the quit
  path writes when the last shell exits) must restore as an EMPTY tree
  (layout_tree::empty_tree) - persist::build_window's old seeded
  new_tree planted an UNREGISTERED pane id (st.panes lookup missed ->
  spawn_pane no-op, screen() respawn branch saw tabs non-empty), so the
  app opened to an empty window forever ("opens to nothing"). do_new_tab
  -> seed_alloc gives the respawned pane a globally-unique id.
- oc gesture drift (3rd round): /usr/local/bin/opencoder (2026-09-16
  build) exits the idle prompt on a SINGLE Ctrl+C. e2e must PROBE (one
  press, wait, escalate only if alive) - a spaced double-tap on a
  single-press build kills the NEXT pane too: auto-close removes the
  corpse, focus containment hands press #2 to the sibling.
- close semantics: closing the LAST pane/tab sets UiState.quitting instead of
  do_new_tab; screen() skips the empty-tabs respawn while quitting so the
  frame can land ViewportCommand::Close (main.rs, every frame; save_if_dirty
  has already persisted). A dead pane (no session yet or exit.is_some())
  closes on a plain click and `tracking = !dead && ...` - stale DEC mouse
  modes must not eat the click. persist: per-tab focused containment (the
  global id remap can resolve a stale focused id into ANOTHER tab -> keys
  leak cross-tab) and active_tab forced to 0 on restore.
- state.json carries `settings {split_axis:"v"|"h", split_ratio 0.05..0.95}`
  (serde default: old files load unchanged). Ctrl+Shift+D splits along
  settings.split_axis; layout-tree split_pane_ratio clamps non-finite ->
  0.5 -> 0.05..0.95.
- app chrome colors derive from the palette in render/colors.rs (mix():
  chrome_bg 4.5% bg->fg, hover 10%, hairline 9%, divider 13%, tab_active
  bg->highlight 18%, title_text fg->bg 42%). ui/style.rs sync() installs a
  dark egui style ONCE per theme change (UiState.styled_theme guards);
  pinned test values: divider(dracula) == (67,69,78), hairline ==
  (59,61,71), tab_active == (67,61,89). egui 0.36 has no ctx.set_style -
  use set_theme(Theme::Dark) + set_style_of.

## Multi-window model (2026-09)
- one root pass renders windows[0]; `windows::render_secondaries` drives
  each secondary via `ctx.show_viewport_immediate(ViewportId(Id::new(id)),
  builder, cb)` - eframe native wgpu registers the immediate-viewport
  renderer so these ARE real OS windows (no backend falls back to
  embedded). The callback gets `&mut Ui` and runs synchronously INSIDE
  that viewport's own input pass: keyboard::handle/pointer events are
  per-viewport; key events only reach the X-focused viewport.
- window ids start at 1 (ViewportId(0)=ROOT; Id::new hashes the u64);
  pane ids stay globally unique via seed_alloc/collect_alloc - a spawned
  window's tree drops the new_tree seed tab (pane id 1 would collide)
  then ensures_next_pane_id + new_tab (same trick as persist
  build_window).
- empty ACTIVE tree: root (idx 0) respawns a tab or quits when it is the
  LAST window (quitting flag); a secondary removes itself
  (actions::handle_empty_window; terminate+panes.remove+retarget focus).
  WM-close of a secondary = remove that window only; secondary close
  semantics tested live. Ctrl+Shift+Q from anywhere quits the whole app.
- `st.active` = window being rendered (set inside windows::render);
  `st.focus` = user-focused window (viewport focused==Some(true)); the
  IPC drain runs on `active = focus` between render passes.
- inspector is PER-WINDOW (WindowUi.inspector): each window's pass draws
  its own panel (root: main.rs after render(0); secondary: inside the
  show_viewport_immediate callback - the callback runs as that viewport's
  own pass, so egui::Window layers land THERE and the font slider writes
  windows[idx]). Two open inspectors coexist via Id salted with win_id.
- windows::render bails early when its window vanished mid-pass
  (keyboard action removed it) - never render the NEXT window's tree
  into the dying viewport. Secondary viewport destruction lags a few
  seconds behind (eframe GC) - e2e polls for the X window to disappear.
- remote desktop (WM present): `xdotool windowfocus` is NOT enough for
  key delivery - `xdotool windowactivate` (EWMH _NET_ACTIVE_WINDOW) is
  required; on bare Xvfb (no WM) windowfocus works (XSetInputFocus).
  `import -window WID` screenshots a 32-bit ARGB window as ALL BLACK -
  screenshot `import -window root` and read the window's pixels from the
  full image instead.

## Verified end-to-end (final state)
All suites green (ui-style, mouse-key, windows W1-W6, ipc-oc, cjk,
oc-exit, remote); Xvfb scripts export TERMINATOR_OPAQUE=1; live-verified
on deploy target 192.168.31.196 (2 X windows, per-window typing
isolation, window close keeps the app, opacity 1.0 = mocha bg not
black, ctl list/capture; remote px differ only via wallpaper blend).
- rendering: catppuccin + ANSI 256 bg exact px, CJK cmap (Han/kana/
  hangul) + real Han ink px, single tab = zero chrome (header y=0),
  pane-title centering (ui-style G), no window-title row (H)
- windows/lifecycle: Ctrl+Shift+N second OS window, last-pane close
  removes the window, root last-pane close with sibling alive respawns a
  tab, Ctrl+Shift+W on non-last pane keeps app alive (K4), dead-pane
  click closes only it (K6), Ctrl+Shift+Q quits from any window, WM
  close honored
- input: key echo, ^C echo through the egui Copy-fold gate, Ctrl+Shift+E
  split, SGR press/release/motion + wheel byte-exact (less wheel =
  arrows x3), cross-pane drag RELEASE to the press-owner pane, drag-
  select + Ctrl+Shift+C == xclip readback, Shift+drag escape hatch,
  Shift+PageUp/End scrollback paging + snap-back-on-typing, Ctrl+C
  interrupts a foreground job (pty signal reset verified)
- opencode panes: real opencoder Ctrl+D/Ctrl+C exits the idle prompt
  (status 0), Ctrl+Shift+W closes a live TUI pane
- control channel: ctl list/capture/send over the live socket (PaneInfo
  `window` u64 serde-default 0, WIN column after ID - column-count
  parsers must adapt), TERMINATOR_SOCK in pane env, oc link via /proc fd
  discovery, submit -> pending -> consume -> receipt by seq, honest
  --wait, SIGTERM-stale socket reclaim (perms 600 on first bind AND
  after reclaim)
- remote zellij: bootstrap create+attach, reconnect, exit-42 degrade;
  gate = loading cleared + typed marker round-trip + session listed
  ("ZELLIJ" never renders - chrome-free config)
- persistence: state.json save/restore across restart
