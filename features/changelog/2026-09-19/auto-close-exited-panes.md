# Auto-close panes whose shell exited (+ pty fork hardening)

Date: 2026-09-19 · Status: verified (unit + e2e)

## Problem
A pane whose child exited stayed as a dead corpse until the user
clicked it (or Ctrl+Shift+W). Users read "shell exited -> click
somewhere -> whole app quits" because the corpse click was the LAST
pane close, which quits the app. Same root cause made `exit` in the
only shell feel like a crash.

## Change
- `vt-pane/session_map.rs`: `SessionMap.exited_seen: HashMap<PaneId,
  Instant>` records when an exit was first seen;
  `EXIT_GRACE = 250ms`; cleared on spawn/respawn/terminate.
- `app/actions.rs`: pure predicate `exit_ripe` (exit set, not the
  exit-42 degrade marker, grace elapsed, session exists - spawn-backoff
  panes are not corpses) + `close_exited(st, sess, ui, dirty)` - each
  frame it finds the first ripe pane across ALL windows/tabs and closes
  it through the normal `do_close_pane` path (so last-pane semantics
  reuse everything: root+only-window -> quit, secondary -> window
  removal, root-with-sibling -> fresh tab respawn).
- `app/render/screen.rs`: `close_exited` runs after `ensure_sessions`;
  a mid-pass window removal (win_id changed) aborts the render pass.
- Grace exists so an instantly-dying shell cannot fork-loop the
  empty-tree respawn; 250ms still lets the last output frame land.
- exit 42 (`EXIT_NO_ZELLIJ`) is left to `auto_degrade` (degrade to
  plain shell, do not close).

## pty fork hardening (root-causes a test flake)
`vt-pane/src/pty.rs`:
- ALL allocations (argv/env pointer tables, PATH resolution) now run
  BEFORE `fork()`; the child only calls async-signal-safe functions
  (setsid/TIOCSCTTY/dup2/close/signal/execve/_exit). The old comment
  claimed a single-threaded fork; false once any reader thread exists.
- `openpty()` replaced by `posix_openpt/grantpt/unlockpt/ptsname_r/
  open` with `O_CLOEXEC` on BOTH ends. Previously a sibling thread
  forking during our open(slave)->fork window leaked our slave into a
  foreign child; that holder kept our master from ever seeing EOF, so
  the exit was never detected ("test session never exited" flakes).
  In production children also no longer inherit sibling masters.
  `dup2` clears FD_CLOEXEC, so the child's stdio survives exec.

## e2e
- `e2e-oc-exit.sh`: K2/K3 double as auto-close assertions (pane leaves
  `ctl list` with no click; sibling + app alive). Gesture drift bit
  AGAIN: the current `/usr/local/bin/opencoder` build exits the idle
  prompt on a SINGLE Ctrl+C - a blind double-tap killed the next pane
  too (auto-close + focus containment hands press #2 to the sibling).
  K2 now probes (press, wait, escalate only if alive); K3 Ctrl+Ds the
  focused survivor and identifies the victim by pid; K4 opens a
  sibling tab (Ctrl+Shift+T) + Ctrl+Prior before Ctrl+Shift+W so the
  last-pane-quit path is not taken. Old K6 (click closes corpse)
  deleted - unreachable by design now.
- `e2e-windows.sh` W1-W6: green, unchanged.

## Tests
- 5 new unit tests in `actions.rs::close_exited_tests` (grace wait,
  sibling kept, last-pane quit, exit-42 carve-out, secondary removal).
- `cargo test --workspace` stable across 8 consecutive runs (the fork
  flake previously hit ~50% of runs).

## Follow-up (2026-09-20): empty-window restore respawned nothing
Symptom: an app whose state.json held `windows:[{tabs:[]}]` (what this
feature itself writes when the last shell exits and the app quits)
opened to an EMPTY window - no pane, no session, `ctl list` empty.
Root cause: `persist::build_window` on an empty tab list returned a
SEEDED `new_tree("shell")` (one tab, pane id 1) - but that path never
registers the pane in `st.panes`. The screen() respawn branch saw
`tabs` non-empty and skipped; `ensure_sessions` then hit
`spawn_pane -> st.panes.get(&1) == None` and silently no-op'd. A ghost
pane id with no metadata and no session - the "opens to nothing"
window (this was very likely the user-visible startup bug all along:
once ANY run wrote the empty state, every start rendered nothing).
Fix: `layout_tree::empty_tree()` (empty tabs, fresh allocator) and
`build_window` returns it for empty tab lists; the ordinary respawn
path (`screen -> do_new_tab -> seed_alloc`) opens a real tab with a
globally-unique pane id and proper registration. Unit test
`empty_window_restores_as_empty_tree_for_respawn` + live repro
verified locally and on the deploy target (restart from the empty
state now lists a live shell pane within 1s).
