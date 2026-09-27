Commit: e4b23d756b53dbf71970da47f8a64e0db3bf27b0

# Tab notices for agent attention

`terminator-ctl notice` now marks the calling pane's inactive tab with one blue
dot. The terminal exports `TERMINATOR_PANE_ID` and, when available, the sibling
`TERMINATOR_CTL` path to pane children. An optional pane argument lets callers
target a pane explicitly. Repeated notices collapse to one dot; focusing the
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
