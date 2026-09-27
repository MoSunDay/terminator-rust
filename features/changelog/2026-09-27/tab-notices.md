Commit: 5c8147f6068dcb1bfb21990043a3d5990b32dd20

# Tab notices for agent attention

`terminator-ctl notice` now marks the calling pane's inactive tab with one blue
dot. The terminal exports `TERMINATOR_PANE_ID` and, when available, the sibling
`TERMINATOR_CTL` path to pane children. An optional pane argument lets callers
target a pane explicitly. Repeated notices collapse to one dot; focusing the
tab acknowledges it, and closing a pane discards its notice.

## Verification

- `cargo test --offline -p ipc-proto` — 10 passed.
- `cargo test --offline -p ctl` — 38 passed.
- `cargo test --offline -p app` — 178 passed, 1 ignored.
- Focused pane environment test in `vt-pane` — passed.
- `cargo clippy --offline -p app -p ctl -p ipc-proto -p vt-pane --all-targets -- -D warnings` — passed.
- `cargo build --offline -p app -p ctl` — passed.
