Commit: 4224d64e16b9e6f7da6c6bdb8352d65fabc1e3c5

# Tab notices for agent attention

`terminator-ctl notice` now marks the calling pane's inactive tab with one blue
dot. The terminal exports `TERMINATOR_PANE_ID` and, when available, the sibling
`TERMINATOR_CTL` path to pane children. (Superseded 2026-09-27: the pane
argument and `$TERMINATOR_PANE_ID` routing are gone — `terminator-ctl notice`
takes no arguments and writes the canonical OSC 9 bytes to the pane's own
tty; see remote-session-notice.md.) Repeated notices collapse to one dot; focusing the
tab in the focused window acknowledges it, and closing a pane discards its
notice. A selected tab in an unfocused window still shows its dot until that
window is focused.

## Verification

- `cargo test --offline -p ipc-proto` — 10 passed.
- `cargo test --offline -p ctl` — 38 passed.
- `cargo test --offline -p app --bin terminator-rust` — 183 passed.
- Focused pane environment test in `vt-pane` — passed.
- `cargo clippy --offline -p app -p ctl -p ipc-proto -p vt-pane --all-targets -- -D warnings` — passed.
- `cargo build --offline -p app -p ctl` — passed.
- `cargo clippy --offline -p app --all-targets -- -D warnings` — passed after
  the background-window badge fix.
