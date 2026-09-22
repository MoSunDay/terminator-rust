# Full-workspace panic & logic audit (17 fixes)

## Problem
A full-repo diagnostic sweep (all crates, reachable-panic + logic-flaw
audit, 25 findings) surfaced 2 P0 runtime panics, 4 P1 correctness/
durability flaws and 11 P2 hardening gaps on top of the earlier GPU
surface-size crash fix. Release users could abort the app or corrupt
state on edge inputs instead of degrading gracefully.

## Fix (per finding)
- P0 `render/screen.rs` viewport bar: `(h*ratio).clamp(12, h)` inverted
  (panics) when a tiny pane's content height < 12 -> `h.min(12.0)` floor.
- P0 `ctl` non-UTF-8 argv: `std::env::args()` panics on lossy bytes ->
  `args_os` + `to_string_lossy` (lossy name falls into not-found).
- P1 `actions` auto_degrade respawn now goes through
  `session_map::note_spawned` so the reconnect backoff ladder (1/2/4/5s)
  resets instead of being pinned at 1s by a stale `spawned_at`.
- P1 `vt-pane/task.rs`: all fallible construction (CStrings, threads)
  moved before/into `build_session`; the reader thread spawns LAST; every
  Err path funnels through one `kill_pty_child` (SIGKILL + close master +
  blocking waitpid, ECHILD tolerated) - no leaked children or master fds.
- P1 `persist.rs`: a state.json that reads but fails to parse is first
  copied to `state.json.corrupt` (fixed name, best-effort) before the
  fresh fallback; save writes `state.json.tmp<pid>` + `sync_all` then
  renames, so multi-instance runs never cross-write tmps and a crash
  never leaves an empty state.json.
- P1 pane rename now rejects `--`-prefixed titles (an untagged
  `PaneSelector` would eat them as flags), mirroring the digits-only
  rule; refusal reason shown in the editor.
- P2 `exit_ripe` narrowed: exit 42 auto-close exemption applies only to
  a REMOTE, not-yet-degraded pane; local/degraded 42 closes like any
  dead shell, connection-drop exits stay with reconnect (new tests).
- P2 `persist::valid_windows`: window ids must be in [1, u64::MAX) and
  unique; invalid windows dropped, all-invalid falls back to fresh
  restore (no ViewportId::ROOT collision, no overflow on max+1).
- P2 `vt-pane/pty.rs`: `CString::new` + argv/env/PATH tables fully built
  BEFORE `open_pty_pair` - a NUL argument now fails before any fd opens;
  the fork child branch still reads only prebuilt tables
  (async-signal-safe invariant kept).
- P2 `mouse.rs`: `clamp_grid_px` floors cols/rows at 1.0 (0-size grid
  made the first mouse event clamp(min>max)); `pos_finite` early-returns
  on NaN/Inf at all 4 public pointer entrypoints (Zig safe builds trap
  on NaN->int across the FFI).
- P2 `render/screen.rs`: per-pane window-liveness re-check inside the
  render loop (a mid-loop close no longer paints another window's tree
  into the dying viewport).
- P2 `pane_header.rs`: title right bound `max(right-usage, left+8s)` -
  a wide badge on a narrow pane can no longer build a negative-size
  title Rect.
- P2 `ctl` column widths via `chars().count()` (matches `{:<w$}`
  padding); `/proc` fd discovery trims a trailing " (deleted)" so an
  unlinked-but-open opencoder.db is still found (tests pin no false
  match on `*-journal`/`other.db`).
- P2 `ctl/sidecar.rs`: 1 MiB read cap (fast fail over parsing a giant
  file) + tmp suffix `pid.atomic-seq` (unique across
  processes/threads) -> concurrent `oc link` never tears the JSON
  (concurrency test added).
- `persist.rs` test module split out to `persist/tests.rs` (703+360
  lines, both under limits); `agents.md` records the clamp/args_os
  panic families as hard-won facts.

## Verified
- `cargo fmt --all --check` clean; `cargo clippy --workspace
  --all-targets -- -D warnings` clean; `cargo test --workspace`
  321 passed / 0 failed; `cargo build --release --workspace` green.
- Pixel e2e on this exact tree: `e2e-ui-style.sh` (A-J incl. font
  scale), `e2e-windows.sh` (W1-W6), `e2e-mouse-key.sh` (T1-T4) all
  green - zero disturbance to normal-size render/input paths.

## Deferred (8 P2/P3, documented)
IPC serve thread + client connect timeout; serde recursion cap for deep
trees; pty write TOCTOU; registry same-identity silent overwrite;
reconnect palette not following theme; sun_path>107B short-path
fallback; ULID same-millisecond monotonicity; chip editor width
unification. Legacy `--`-prefixed pane names in old state.json stay
unaddressable by ctl (manual rename; no migration).
