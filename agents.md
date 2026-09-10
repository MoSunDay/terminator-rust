Commit: 4ebb8eba52338c50a56abdacc9eed9b37b401f1f

# agents.md - repo memory for terminator-rust

Pure-functional Rust (no classes) terminal multiplexer: egui 0.36 front-end
+ ghostty VT engine. One file <= 400 lines. No unwrap outside tests.

## Crate map
- layout-tree: tab/pane tree, splits, focus, `layout_tab` geometry
- vt-pane: PTY sessions; `spawn_session/pump/frame/resize/send_key/paste`
- theme: palettes + xterm 256 cube + `blend_background`
- remote: ssh -tt + zellij bootstrap (exit 42 = no zellij -> degrade)
- ipc-proto: serde wire types for the UDS control socket (Request/Response)
- ctl: `terminator-ctl` CLI: list/capture/send + `oc` link/submit/status/
  sessions; /proc discovery of the pane's opencoder process
- oc-store: direct rusqlite access to opencoder per-workdir stores
  (schema guard v18, insert/pending/receipts; `oc-store-fixture` dev bin)
- app: egui UI; binary `terminator-rust`; state at
  ~/.config/terminator-rust/state.json; UDS ipc in src/ipc/

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
- opencoder exit e2e: `scripts/bin/e2e-oc-exit.sh` (Xvfb + REAL
  /root/opencoder binary; OC_BIN override; SHELL wrapper that `exec`s the
  binary so pane pid == opencoder pid -> `kill -0` is exit ground truth; dummy
  ~/.opencoder/config.json needed - onboarding form eats ^C; focus navigation
  via Ctrl+Shift+Right, NEVER clicks into mouse-tracking panes)
- cjk font e2e: `scripts/bin/e2e-cjk.sh` (fontTools cmap coverage of the
  embedded subset; Xvfb live app: ctl send `echo 汉字测试` capture round-trip
  + scrot/PIL connected-ink assertions - real Han ink ~15x17px with interior
  strokes; ASCII <=9x10.5, tofu replacement square is hollow inside)
- ui style e2e: `scripts/bin/e2e-ui-style.sh` (Xvfb + scrot/PIL pixel
  assertions: pane bg/theme blend via transparency, gutter two-tone
  (chrome_bg field + 1px divider line), chrome top-bar fill, active-chip
  underline + fill; expectations are COMPUTED in-script from dracula
  constants via a mix() helper - token retunes touch only colors.rs,
  geometry changes touch the script; presets a dracula Split state.json)
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
  network and gives the idle prompt where ^C exits.
- shortcuts: Ctrl+Shift+Q = global quit (Action::Quit -> egui
  ViewportCommand::Close; to_skey must map Key::Q); pane title centering
  (pane_header.rs draws at title_rect.center() CENTER_CENTER) is
  pixel-asserted by e2e-ui-style.sh check G; the chrome is a SINGLE row -
  the old centered window-title row is deleted (it duplicated the tab name
  a third time under the chip and the pane header); zoom/inspector cells
  are anchored to the chip row's right edge via ui.interact on fixed rects
  and check H asserts title-text ABSENCE in the bare chrome zone; rename
  editors (pane + tab) cancel on outside click via `i.pointer.any_click()
  && interact_pos().is_none_or(...)` so a miss-click cannot strand the
  editor.

- CJK display: egui default fonts have ZERO CJK glyphs -> Chinese was tofu.
  Fix: assets/fonts/NotoSansSC-Regular-subset.otf (9.8MB OFL subset, extracted
  SC face idx 2 from Debian NotoSansCJK ttc, pyftsubset terminal ranges +
  Hangul AC00-D7AF, 40330 glyphs; the recipe NEEDS --no-layout-closure or
  GSUB closure inflates it to 47k glyphs; Hangul glyphs are EAW-wide at
  0.92em advance and paint slightly inside the 2-cell pair)
  include_bytes! + FontDefinitions families[Monospace/Proportional].push
  (family list IS the glyph fallback chain); install once in Terminator::new
  via ctx.set_fonts. TERMINATOR_CJK_FONT=path[:ttc_index] swaps the embedded
  bytes (last ':' + u32-suffix = face index). Han advance is 1.0em vs 'M'
  ~0.6em, so wide cells paint at CellSize.wide_size = font_size * 2w/adv("汉"
  via layout_no_wrap, ~1.204, clamp 0.8..=1.8 else 1.2) - cell PITCH is set by
  grid geometry either way; env fonts are parse-validated at startup
  (skrifa FontRef::from_index - epaint's own parser, direct workspace dep):
  unreadable OR unparseable -> log::warn + embedded fallback, never the
  epaint first-layout panic (app always starts). e2e ink detection: wide-glyph ink overflows the
  ASCII line height (1.2x) and stacked lines TOUCH -> row-band splitting is
  impossible; use (column-run x row-run) components and expect >=4 wide+>=3
  stroked (a tofu fallback square passes width but has a hollow interior).
  Xvfb in scripts: NEVER derive the display number from $$ (stale
  /tmp/.X11-unix sockets make Xvfb refuse to bind) - probe random free
  numbers, and `export DISPLAY` for xdotool/scrot (env DISPLAY=... on the app
  line alone is not enough).
## Verified end-to-end (2026-09)
Xvfb: render + catppuccin colors exact px, key echo, ANSI 256 bg exact
px, Ctrl+Shift+E split, state.json save/restore across restart, WM close.
Batch-1 mouse (Xvfb): SGR press/release/motion + wheel press-only
reports byte-exact, less wheel = arrows x3, viewport scrollback +
snap-back-on-typing, drag-select highlight + Ctrl+Shift+C == xclip
readback, Shift+drag escape hatch.
ssh-localhost: zellij create+attach via bootstrap, exit-42 degrade,
reconnect to live session. e2e gate: bootstrap config is chrome-free so
"ZELLIJ" NEVER renders; gate = loading screen cleared + typed marker
(zellij round-trip) + session in list-sessions.
Control channel (2026-09, scripts/bin/e2e-ipc-oc.sh): ctl list/capture/send
over the live socket, TERMINATOR_SOCK present in the pane child env, oc link
via /proc fd discovery, submit -> pending -> consume -> receipt by seq,
honest --wait timeout, second instance reclaims a SIGTERM-stale socket
(socket perms asserted 600 on first bind AND after reclaim).
Mouse/key review fixes (2026-09, scripts/bin/e2e-mouse-key.sh): ^C echo
through the egui Copy-fold gate, cross-pane drag RELEASE delivered to the
press-owner pane's tracker child, Shift+PageUp/End scrollback paging, and
Ctrl+C interrupting a foreground sleep 45 (pty signal reset verified).
- state.json now carries `settings {split_axis:"v"|"h", split_ratio
  0.05..0.95}` (serde default: old files load unchanged). Ctrl+Shift+D
  splits along settings.split_axis; layout-tree split_pane_ratio clamps
  non-finite -> 0.5 -> 0.05..0.95.
- app chrome colors derive from the palette in render/colors.rs (mix():
  chrome_bg 4.5% bg->fg, hover 10%, hairline 9%, divider 13%, tab_active
  bg->highlight 18%, title_text fg->bg 42%). ui/style.rs sync() installs a
  dark egui style ONCE per theme change (UiState.styled_theme guards);
  pinned test values: divider(dracula) == (67,69,78), hairline ==
  (59,61,71), tab_active == (67,61,89). egui 0.36 has no ctx.set_style -
  use set_theme(Theme::Dark) + set_style_of.
opencoder exit + quit (2026-09, scripts/bin/e2e-oc-exit.sh): real opencoder
panes Ctrl+D/Ctrl+C exit (status 0), Ctrl+Shift+W closes a live TUI pane,
Ctrl+Shift+Q quits the app; e2e-ui-style.sh G asserts pane-title pixel
centering and H the absence of the removed window-title row. Deployed to
192.168.31.196 and verified live (ctl list/capture; remote px differ from
Xvfb only via wallpaper transparency blend).
