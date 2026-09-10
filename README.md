# terminator-rust

A terminator-style terminal multiplexer built with
[egui](https://github.com/emilk/egui)/eframe and the
[ghostty](https://github.com/ghostty-org/ghostty) VT engine
(`libghostty-vt`, consumed via `libghostty-rs`).

![status](https://img.shields.io/badge/build-passing-green) pure-functional Rust, no classes.

## Features

- Tabs and recursive pane splits (horizontal / vertical), terminator-style
  shortcuts, drag dividers, per-tab zoom.
- Per-pane overrides: custom background color, transparency, manual titles.
- Theme packs: catppuccin-mocha, tokyo-night, dracula, gruvbox-dark,
  terminator-classic; ANSI 256-color cube support.
- Invisible remote sessions: `ssh -tt` + auto-bootstrapped `zellij`
  (`~/.cache/zt`) with keybinds cleared and frames off, so the remote
  multiplexer never fights the local one. Reconnect on drop; graceful
  degrade to plain ssh (exit 42 marker) when zellij is missing.
- Layout and pane state persisted to `~/.config/terminator-rust/state.json`;
  remote hosts registry at `~/.config/terminator-rust/sessions.json`.

## Fonts / Unicode

The binary embeds a Noto Sans SC subset (OFL, `assets/fonts/OFL.txt`) as
the last entry of every font fallback chain, so Han, kana, bopomofo,
fullwidth forms, roman numerals, circled digits AND Hangul syllables
(AC00-D7AF) render with zero system-font dependency, so Linux and macOS
output is identical by construction. Override with
`TERMINATOR_CJK_FONT=path[:face_index]` to swap in any system font, e.g.
a full `NotoSansCJK.ttc` face for Traditional/Korean-preferred glyph
variants or Ext-B coverage. The override is parse-validated at startup
and falls back to the embedded font with a warning if the file is
unreadable or not a valid font.

## Layout

| crate        | role                                                    |
| ------------ | ------------------------------------------------------- |
| `layout-tree`| tab/pane tree, splits, focus, divider geometry          |
| `vt-pane`    | PTY + ghostty VT session (fork/exec, resize, key encode)|
| `theme`      | palettes, xterm 256 color, background blending          |
| `remote`     | ssh/zellij bootstrap, registry, reconnect plans         |
| `app`        | egui front-end (rendering, input, tabs, headers, UI)    |

`third_party/libghostty-vt/` holds a prebuilt static ghostty VT library
(gitignored; see `third_party/README.md`).

## Build

Requirements: Rust stable, a C compiler, `pkg-config`. The vendored
ghostty static lib is rebuilt with zig automatically when missing:

```sh
cargo build --workspace
cargo test --workspace
# optional e2e (needs local sshd + zellij):
cargo test -p remote --test zellij_e2e -- --ignored
```

Run: `cargo run -p app` (binary `terminator-rust`).

## Shortcuts

| keys                | action                       |
| ------------------- | ---------------------------- |
| Ctrl+Shift+T        | new tab                      |
| Ctrl+Shift+W        | close pane                   |
| Ctrl+Shift+O / E    | split horizontal / vertical  |
| Ctrl+Tab / +Shift   | cycle panes                  |
| Ctrl+Shift+Arrows   | move focus                   |
| Ctrl+PageUp / Down  | prev / next tab              |
| Ctrl+Shift+F        | zoom focused pane            |
| Ctrl+Shift+R        | respawn / reconnect pane     |
| Ctrl+Shift+V        | paste into pane              |

Tab bar: middle-click closes, double-click renames. Pane header:
double-click renames, swatch button sets color, moon button sets
transparency, right-click for context menu.

## Non-goals (v1)

Scrollback scrollbar UI, kitty graphics protocol, window transparency /
blur, Windows support.

## macOS notes

The codebase is POSIX (openpty/fork/exec); macOS ships all of it. To build
there, re-vendor the ghostty static lib for `aarch64-apple-darwin`
(`scripts/fetch-vendor.sh` with a native zig) or point `PKG_CONFIG_PATH`
at a homebrew ghostty. No code changes expected; `libc` handles the
platform calls. Not yet CI-verified.
