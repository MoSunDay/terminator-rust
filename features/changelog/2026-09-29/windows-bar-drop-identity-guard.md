Commit: 1b6d59b871dc0c220cd97664e3a9b2f905b11264

# Bar-stage identity guard: a dying viewport must never render another window's tree

## Root cause

Releasing a cross-window tab-chip drag runs in the SOURCE window's own
bar pass (drag state lives on its `WindowUi`, release truth comes from
the X11 pointer poll). When that tab was the source window's last, the
release drops the source window itself (`drop_empty_window` ->
`st.windows.remove` + `retarget_after_remove`). With >= 3 windows,
`retarget_after_remove` clamps `active` to `min(src, n-1)`, which is
NOT the drop destination: the dying viewport pass then rendered
whichever window now sat at that index — an INNOCENT window's tree was
laid out into the dying viewport, and `sync_frame` TIOCSWINSZ'd its
tmux/ssh panes to the dying geometry for one-two frames (flick-wrong-
then-snap-back; a long-lived ssh pane sees its rows/cols thrash).
Two windows never showed it: the retarget slot coincided with the
destination AND equal default grids made the spurious resize a no-op.

## Fix

`crates/app/src/windows.rs::render` re-checks window identity AFTER
the `tab_bar` panel, mirroring the pre-bar guard: when the window at
this index is no longer this pass's window, return before
`CentralPanel`/`screen()` — no pane sync, no IME sync, no foreign
tree in the dying viewport.

## Reproduce / verify

```sh
cargo build --workspace
./scripts/bin/e2e-windows.sh   # W11
```

W11 (added in the same commit): three windows, the middle window
externally sized so its grid differs from the victim's, a python
SIGWINCH watcher (`signal.signal` + print, injected via
`terminator-ctl send`, so no window focus is needed) in the victim
pane. Drag the middle window's only tab onto the root strip. Gates:
- the victim pane NEVER sees a SIGWINCH — a dash `trap` is NOT a
  probe: it defers/loses WINCH while waiting in a sleep loop, python
  handlers run promptly mid-sleep;
- every pane keeps its child pid, the moved tab lands in the root
  window, the source window disappears, the app stays alive.

Verified in BOTH directions: guard disabled -> the same test FAILS
(`unrelated window's pane was resized by the W11 drop`), with
instrumentation counting 3 TIOCSWINSZ on the victim (resized
100x26 -> 66x19 twice plus one correction); guard enabled -> PASS.
