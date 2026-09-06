Commit: 947713b724383b4b4ffb6a3f6e21f29c5662149e

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

## Hard-won facts (do not relearn)
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
- copy path: egui-winit folds ctrl/cmd+C/X/V (shift variants and dedicated
  keys too) into Event::Copy/Cut/Paste and emits NO Key event; keyboard.rs
  gates on ctrl&&!shift: bare Ctrl+C/X/V forwards ^C(SIGINT)/^X/^V to the
  child, other forms -> actions::copy_focused (arboard, skip empty); follow_output(s) after keys/paste so typing snaps
  scrollback to live. context_menu suppressed while tracking.
- pointer releases follow the press owner (uist.pointer_pane implicit
  grab) so SGR children never miss a release; app surface_px multiplies
  egui points by pixels_per_point (vt-pane cell px are physical).
- Xvfb smoke ops: launch with `setsid nohup ... </dev/null &` to survive
  across tool calls; NEVER `pkill -f` a pattern that occurs in your own
  command line (self-kill, tool exit -1) - kill by PID instead.
  Ground truth: select text + Ctrl+Shift+C then `xclip -o -selection
  clipboard`; pixels via scrot + PIL.
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
