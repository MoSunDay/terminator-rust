Commit: dfac4c0272e337805f3954d3f431ad8473c052c9

# Terminal TUI ink and motion

## Context

Codex CLI displayed fewer visual distinctions in a Terminator pane than
the same tmux session in iTerm. The VT snapshot kept bold and italic but
discarded faint, blink, hidden, and strikethrough. The painter ignored
bold; terminal output was sampled on a fixed 50 ms cadence.

## Change

- Retain the missing VT style flags in `CellData` and paint faint,
  blink, hidden, strikethrough, and a stronger bold glyph.
- Use a shared blink phase for all panes, including unfocused panes.
- Repaint 16 ms after frames with recent PTY output, then return to the
  50 ms idle cadence after 500 ms.

## Verification

`cargo test -p vt-pane -p app --quiet` and `cargo build -p app --quiet`
passed. A headless painter test verifies the visible ink shapes and faint
color. A live Xvfb probe could not complete on this host: its X11 runtime
lacks `libxkbcommon-x11.so`, and supplying that library temporarily still
left wgpu unable to create a surface. Italic uses epaint's glyph slant.
