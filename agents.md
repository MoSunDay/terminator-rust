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
- libghostty default palette green ~(181,189,104) red ~(224,108,117) -
  color tests assert dominance, not VGA values.
- control socket: $XDG_RUNTIME_DIR/terminator-rust/ipc.sock (fallback
  ~/.config/terminator-rust/); start() live-probes and reclaims stale files
  (SIGTERM runs no destructors, the file survives; next start removes it);
  pane children inherit TERMINATOR_SOCK; manual pane titles are unique
  addressing keys - pane rename rejects duplicates.
- oc submit writes session_inputs (delivery steer|queue) straight into the
  opencoder store (schema guard PRAGMA user_version=18, BEGIN IMMEDIATE,
  admitted_seq = MAX+1); the opencoder TUI is the SOLE runner - rows drain
  only at its turn boundaries (steer) / idle (queue); a TUI left idle
  strands pending rows until its next interaction (their idle_rekick hook
  exists but nothing polls it; --wait reports timeout honestly).

## Verified end-to-end (2026-09)
Xvfb: render + catppuccin colors exact px, key echo, ANSI 256 bg exact
px, Ctrl+Shift+E split, state.json save/restore across restart, WM close.
ssh-localhost: zellij create+attach via bootstrap, exit-42 degrade,
reconnect to live session. e2e gate: bootstrap config is chrome-free so
"ZELLIJ" NEVER renders; gate = loading screen cleared + typed marker
(zellij round-trip) + session in list-sessions.
Control channel (2026-09, scripts/bin/e2e-ipc-oc.sh): ctl list/capture/send
over the live socket, TERMINATOR_SOCK present in the pane child env, oc link
via /proc fd discovery, submit -> pending -> consume -> receipt by seq,
honest --wait timeout, second instance reclaims a SIGTERM-stale socket.
