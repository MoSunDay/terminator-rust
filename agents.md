# agents.md - repo memory for terminator-rust

Pure-functional Rust (no classes) terminal multiplexer: egui 0.36 front-end
+ ghostty VT engine. One file <= 400 lines. No unwrap outside tests.

## Crate map
- layout-tree: tab/pane tree, splits, focus, `layout_tab` geometry
- vt-pane: PTY sessions; `spawn_session/pump/frame/resize/send_key/paste`
- theme: palettes + xterm 256 cube + `blend_background`
- remote: ssh -tt + zellij bootstrap (exit 42 = no zellij -> degrade)
- app: egui UI; binary `terminator-rust`; state at
  ~/.config/terminator-rust/state.json

## Build/e2e
- `cargo build/test --workspace` (PKG_CONFIG_PATH set by .cargo/config.toml)
- headless UI smoke: Xvfb :NN + xdotool (type into window works; needs
  `xdotool windowfocus` - no WM focus otherwise)
- remote e2e: `cargo test -p remote --test zellij_e2e -- --ignored`
  (needs local sshd key auth + zellij; cleanup uses delete-all-sessions
  --force)

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
  `A..Z`; app/input/keyboard.rs maps them.
- libghostty default palette green ~(181,189,104) red ~(224,108,117) -
  color tests assert dominance, not VGA values.

## Verified end-to-end (2026-09)
Xvfb: render + catppuccin colors exact px, key echo, ANSI 256 bg exact
px, Ctrl+Shift+E split, state.json save/restore across restart, WM close.
ssh-localhost: zellij create+attach via bootstrap, exit-42 degrade,
reconnect to live session.
