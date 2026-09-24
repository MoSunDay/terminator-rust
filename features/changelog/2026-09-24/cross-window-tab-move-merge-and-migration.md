Commit: (working-tree, pre-initial-commit)

# Cross-window tab move/merge + cross-process pane migration

Follow-up to the in-process drag-and-drop work (2026-09-19
`tab-pane-dragdrop.md`, 2026-09-20 cross-tab migration).

## Problem
Tabs were trapped in the OS window that owned them: no way to hand a tab
to a sibling window, no way to collapse every window back into the root
one, and no way at all to move a live pane to a DIFFERENT PROCESS (the
one case where a pane cannot simply change owner in the same `AppState`).

## Change Summary
Three in-process operations plus one cross-process protocol.

1. `Ctrl+Shift+M` = merge: every secondary window's tabs are appended to
   window 0 (the empties removed), the merged-away windows' ids retarget
   focus, and the root window keeps every pane running - no session is
   touched.
2. `Ctrl+Shift+J` = move the ACTIVE tab of the focused window to the next
   window in ring order (root <- last window).
3. Cross-window tab drag: the tab-chip drag that already reordered chips
   now runs the strip hit-test of EVERY window each frame, so a chip can
   be dropped on a sibling window's tab bar (`ui/xdrag.rs` publishes the
   drag through `state/screens.rs::WinScreens`, and the target window
   paints a caret + strip tint).
4. Cross-process migration: `terminator-ctl migrate <pane> --to <sock>`
   moves the pane's WHOLE tab to another running instance over a real
   UDS. The PTY master fd travels via `SCM_RIGHTS`, the ghostty screen
   via a serialized snapshot, and the target adopts both under a fresh
   pane id set.

## Implementation
- State surgery (`actions/winops.rs`): `take_tab` / `drop_empty_window` /
  `do_move_tab_to_window` / `do_merge_windows` / `do_move_tab_next_window`.
  A window is dropped with `st.windows.remove` + `retarget_after_remove`,
  NEVER `windows::remove_window` (that terminates panes).
- Wire (`ipc-proto/src/migrate.rs`): `MigrateTab {title, focused, root}`
  with `MigrateNode::{Pane, Split{axis, ratio, first, second}}`,
  `MigratePane{manual_title, kind, degraded, pid, cols, rows}`,
  `MAX_MIGRATE_PANES=16`, `MAX_MIGRATE_PAYLOAD=32MiB`, plus
  `discover_sockets` for `ctl instances`.
- fd passing (`app/src/ipc/fd.rs`): `send_with_fds`/`recv_with_fds`;
  ancillary data rides the FIRST payload byte only. `Request::MigrateOut
  {pane, target}` / `Request::TabOffer {tab}` / `Response::Migrated
  {panes}`.
- Source side (`app/src/ipc/handle_migrate.rs`): `dup_master` EVERY leaf
  BEFORE the first `stop_reader` (the reader loop is the only closer of
  the master fd), wait for `reader_done`, drain + `snapshot`, then offer.
  On a refused/timed-out offer it ROLLS BACK by re-adopting the dup -
  the child is never signalled. `session_map::detach` is the
  no-signal sibling of `terminate`, used by both sides.
- Target side: `adopt_session` re-creates the terminal, replays the
  snapshot, stores the fd + child pid, and restarts the reader; pane ids
  are re-allocated from `max(next_pane_id, tree)` so the donor's ids
  never collide.

## Impact Surface
- `Ctrl+Shift+M` / `Ctrl+Shift+J` shortcuts (`state/shortcuts.rs`,
  `input/keymap.rs`).
- `terminator-ctl instances [--json] [--all]`, `terminator-ctl migrate`.
- A tab now survives `WindowState` boundaries and process boundaries with
  its pty, its screen and its running child intact.
- 不影响：单窗口路径（M/J 为 no-op）、远端 attach、主题与存储格式不变；
  迁移失败时目标端不落 tab、源端重新收养 dup 的 master fd 并保留 pane，
  子进程零信号。

## Notes / Compatibility
- Migration is deliberately a control-channel operation: the UI trigger
  (drag onto a foreign window) is not wired, so a cross-process move
  cannot deadlock the frame loop on a blocking socket round trip.
- Refused offer / dead target / >16 panes / >32MiB snapshot all fail
  closed: the source keeps the pane exactly as it was.
- `scripts/bin/e2e-migrate.sh` (CI job `e2e-migrate`) drives two real
  instances on one Xvfb and asserts same-pid + same-screen after the
  move, live typing in the adopted pane, the donor's respawn, and the
  child never dying - plus the round trip back.
- `scripts/bin/e2e-windows.sh` W7-W10 cover the in-process trio: the
  shortcut move (secondary -> root), the single-window spawn case, the
  merge, and the chip drag onto a sibling's strip (its tint gate and
  chip scan are theme-agnostic - the default theme is kanagawa-wave, so
  e2e-lib's dracula fill constants would find nothing).

## Also In This Commit
- `crates/app/src/ui/tabs_widgets.rs`: the minimize glyph is drawn on the
  cell's vertical centre (the old y+4 dash read as '_');
  `scripts/bin/e2e-window-controls.sh` R4 now probes the dash ink centroid
  against the band's modal luminance before the iconify/restore leg.
- `scripts/bin/deploy-remote.sh`: restart candidates come from
  `pgrep -f <install path>` AND `pgrep -x terminator-rust` (a panel-launched
  instance carries a bare argv[0]); `/proc/<pid>/exe` under the install root
  stays the only kill gate.
- CI: the multi-window job installs scrot + python3-pil and runs W1-W10; a
  new `e2e-migrate` job runs M1-M5.
- `agents.md` refreshed (TAB MOBILITY entry, W10/CI facts, deploy restart
  candidates, stale TIOCPTYGNAME constant repaired); the verified-state list
  moved to `agents/verified-end-to-end.md` to keep agents.md under 800 lines.

## 测试覆盖 (Test coverage)

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 窗口合并 / tab 迁移（原地） | `merge_windows_folds_all_windows_into_root_in_order`, `merge_windows_is_a_no_op_with_one_window`, `move_tab_to_window_moves_tab_and_keeps_panes`, `move_tab_out_of_root_leaves_root_window_alive`, `move_tab_next_window_detaches_into_fresh_window`, `move_tab_next_window_cycles_to_next_window`, `strip_hit_excludes_source_and_prefers_last` | `crates/app/src/actions/winops_tests.rs` |
| SCM_RIGHTS fd 传递 | `roundtrips_data_and_fds_over_a_socketpair`, `plain_send_carries_no_fds`, `excess_fds_error_after_closing_them`, `oversize_fd_list_is_rejected_upfront` | `crates/app/src/ipc/fd.rs` |
| migrate out / tab offer / 失败回滚 | `migrate_out_hands_the_pty_over_and_never_signals_the_child`, `tab_offer_adopts_the_pty_and_plants_a_new_tab`, `tab_offer_rejects_a_mismatched_fd_count`, `a_refused_offer_restores_the_pane_locally` | `crates/app/src/ipc/migrate_tests.rs` |
| adopt 往返（真 pty + `cat`，含 scrollback） | `adopt_roundtrip_preserves_child_screen_and_scrollback` | `crates/vt-pane/tests/adopt.rs` |
| wire 编解码 + 实例发现 | `request_response_wire_shapes`, `payload_roundtrip_and_layout`, `payload_truncated_or_trailing`, `payload_max_enforced`, `discover_sockets_filters_skips_and_sorts` | `crates/ipc-proto/src/migrate.rs` |

- 全量回归：`cargo test --workspace` → 21 suites ok / 0 failed。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- fmt / build：`cargo fmt --all -- --check`、`cargo build --workspace` → 干净。
- e2e：`scripts/bin/e2e-migrate.sh` → PASS (M1-M5，两个真实实例)；
  `scripts/bin/e2e-windows.sh` → PASS (W1-W10)。
- 行数：`ipc/handle_migrate/{mod,out,in,wire}.rs` 65/275/126/160 ≤ 400；
  `actions/winops.rs` 230 + `winops_tests.rs` 183 ≤ 400；`agents.md` 783 ≤ 800。

## Related Docs
- [agents/verified-end-to-end.md](../../../agents/verified-end-to-end.md)
- [2026-09-19 tab/pane drag-and-drop](../2026-09-19/tab-pane-dragdrop.md)
- [agents.md](../../../agents.md)
