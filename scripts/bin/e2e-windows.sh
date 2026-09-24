#!/usr/bin/env bash
# Headless e2e: multi-OS-window support (egui immediate viewports).
#   W1: Ctrl+Shift+N spawns a second OS window; both X windows exist and
#       both have a live, INDEPENDENT shell pane (globally unique pane ids).
#   W2: typing goes to the focused window only; ctl captures prove the two
#       panes hold different markers.
#   W3: closing the secondary's last pane (Ctrl+Shift+W) removes that OS
#       window; the app and the root window stay alive.
#   W4: a re-spawned secondary works the same way (window ids progress).
#   W5: Ctrl+Shift+Q pressed in the SECONDARY window quits the whole app.
#   W6: closing the ROOT window's last pane while a sibling window lives
#       respawns a fresh root tab (root never dies with siblings) instead
#       of quitting the app.
#   W7: Ctrl+Shift+J in the secondary hands its tab back to the root: the
#       emptied secondary disappears, the moved pane keeps its child pid
#       and becomes the root's active tab.
#   W8: Ctrl+Shift+J with a SINGLE window spawns one for the active tab
#       (seed tab closed through the normal path) - both panes keep their
#       pids and each window types into its own pane.
#   W9: Ctrl+Shift+M merges every window back into the root, panes alive.
#   W10: dragging a chip onto the OTHER window's tab strip moves
#       the tab there (the source window disappears, the pane keeps
#       its pid, the receiver activates it); the receiving strip
#       shows the drop tint while the drag hovers it.
#
# Usage: scripts/bin/e2e-windows.sh  (repo root; needs Xvfb + xdotool).
#        E2E_KEEP=1 keeps the scratch dir for debugging.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-win-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
DISPLAY_N=""
XVFB_PID=""
APP_PID=""

cleanup() {
    [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
    [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
    sleep 0.3
    if [ "${E2E_KEEP:-0}" = "1" ]; then
        echo "(E2E_KEEP=1: scratch dir kept at $ROOT)"
    else
        rm -rf "$ROOT"
    fi
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo "== $*"; }

# pane_ids: all pane ids from `list --json`, ascending.
pane_ids() {
    "$CTL" list --json 2>/dev/null | grep -o '"id": [0-9]*' | grep -o '[0-9]*' | sort -n
}

# pane_pid <pane-id>: the pane's child pid (empty when it is gone).
pane_pid() {
    "$CTL" list --json 2>/dev/null | awk -v want="$1" '
        /"id": / { id = $2; sub(/,.*/, "", id) }
        /"pid": / { pid = $2; sub(/,.*/, "", pid) }
        /"pid": / && id == want { print pid }
    '
}

# pane_win <pane-id>: the OS window id owning the pane ("1" = root).
pane_win() {
    "$CTL" list --json 2>/dev/null | awk -v want="$1" '
        /"id": / { id = $2; sub(/,.*/, "", id) }
        /"window": / && id == want { w = $2; sub(/,.*/, "", w); print w }
    '
}

# win_geom <wid>: "<x> <y> <width> <height>" of an X window (screen coords).
win_geom() {
    xdotool getwindowgeometry --shell "$1" | awk -F= '
        $1 == "X"      { x = $2 }
        $1 == "Y"      { y = $2 }
        $1 == "WIDTH"  { w = $2 }
        $1 == "HEIGHT" { h = $2 }
        END { print x, y, w, h }'
}

# chip_edges <png> <x> <y> <w>: theme-agnostic scan of one window's chip
# row, printing "<left> <right>" of the ACTIVE chip's fill run. Bare
# chrome (far right of the chips) is the reference colour: a chip column
# differs from it across most of the band, and the FIRST wide run of such
# columns (chips are left-aligned) is the chip.
chip_edges() {
    W10_SCAN="$1" W10_X="$2" W10_Y="$3" W10_W="$4" python3 - <<'PYS'
import os
import sys
from PIL import Image

img = Image.open(os.environ["W10_SCAN"]).convert("RGB")
x0, y0, w = (int(os.environ[k]) for k in ("W10_X", "W10_Y", "W10_W"))
rows = list(range(y0 + 8, y0 + 25))
bare = img.getpixel((x0 + w * 2 // 3, y0 + 16))

def differs(x):
    hits = sum(
        1
        for y in rows
        if max(abs(a - b) for a, b in zip(img.getpixel((x, y)), bare)) > 6
    )
    return hits > len(rows) // 2

# Chips are LEFT-aligned, so the chip (one tab here) is the first wide
# run in the row's left half - not the longest one: the trailing chrome
# cells (zoom / settings / + / split / window buttons) can merge into a
# longer run on the right.
runs = []
run = None
for x in range(x0 + 2, x0 + w - 2):
    if differs(x):
        run = x if run is None else run
    elif run is not None:
        runs.append((run, x))
        run = None
if run is not None:
    runs.append((run, x0 + w - 2))
wide = [r for r in runs if r[1] - r[0] >= 40 and r[0] <= x0 + w // 2]
if not wide:
    print(f"no chip run found (runs {runs})", file=sys.stderr)
    sys.exit(1)
left, right = wide[0]
if right - left > 300:
    print(f"chip run too wide ({left}..{right})", file=sys.stderr)
    sys.exit(1)
print(left, right)
PYS
}

# strip_diff <png-a> <png-b> <x0> <x1> <y>: count pixels of the chip-row
# band (y+6..y+40, x0..x1) whose channels differ by more than 6. Used to
# spot the drop tint a receiving window paints under a foreign drag -
# always against a baseline scrot of the same band, so it needs no
# palette constants.
strip_diff() {
    W10_A="$1" W10_B="$2" W10_X0="$3" W10_X1="$4" W10_Y="$5" python3 - <<'PYS'
import os
from PIL import Image

a = Image.open(os.environ["W10_A"]).convert("RGB")
b = Image.open(os.environ["W10_B"]).convert("RGB")
x0, x1, y = (int(os.environ[k]) for k in ("W10_X0", "W10_X1", "W10_Y"))
print(
    sum(
        1
        for yy in range(y + 6, y + 40)
        for x in range(x0, x1)
        if max(abs(p - q) for p, q in zip(a.getpixel((x, yy)), b.getpixel((x, yy)))) > 6
    )
)
PYS
}

# wait_capture <pane-id> <marker> [tries]: until the pane shows the marker.
wait_capture() {
    local pane=$1 marker=$2 tries=${3:-40}
    for _ in $(seq 1 "$tries"); do
        if "$CTL" capture "$pane" 2>/dev/null | grep -q "$marker"; then
            return 0
        fi
        sleep 0.25
    done
    return 1
}

# wait_win <name-regex> [tries] -> echoes the newest matching window id.
wait_win() {
    local pat=$1 tries=${2:-40} ids
    for _ in $(seq 1 "$tries"); do
        ids=$(xdotool search --name "$pat" 2>/dev/null || true)
        if [ -n "$ids" ]; then
            echo "$ids" | tail -1
            return 0
        fi
        sleep 0.25
    done
    return 1
}

wait_win_gone() {
    local pat=$1 tries=${2:-40}
    for _ in $(seq 1 "$tries"); do
        xdotool search --name "$pat" >/dev/null 2>&1 || return 0
        sleep 0.25
    done
    return 1
}

wait_pid_gone() {
    local pid=$1 tries=${2:-40}
    for _ in $(seq 1 "$tries"); do
        kill -0 "$pid" 2>/dev/null || return 0
        sleep 0.25
    done
    return 1
}

cargo build -p app -p ctl --bins >/dev/null

export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1   # pin fades/cursor blink to end states
mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"
# plain sh: no OSC title churn, window names stay deterministic
export SHELL=/bin/sh

# --- Xvfb + app -----------------------------------------------------------
step "launch Xvfb + app"
for _ in $(seq 1 12); do
    N=$((100 + RANDOM % 880))
    [ -S "/tmp/.X11-unix/X$N" ] && continue
    Xvfb ":$N" -screen 0 1400x900x24 & XVFB_PID=$!
    sleep 0.7
    if kill -0 "$XVFB_PID" 2>/dev/null && DISPLAY=":$N" xdpyinfo >/dev/null 2>&1; then
        DISPLAY_N=":$N"
        break
    fi
    kill "$XVFB_PID" 2>/dev/null || true
    XVFB_PID=""
done
[ -n "$DISPLAY_N" ] || fail "no free X display for Xvfb"
export DISPLAY="$DISPLAY_N"
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "control socket never appeared"; }

ROOT_WID=$(wait_win '^terminator-rust$') || { tail "$ROOT/app.log"; fail "root window not found"; }
sleep 1
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done

# --- W1: spawn a second OS window ----------------------------------------
step "W1: Ctrl+Shift+N opens a second OS window"
sleep 1   # let the root shell settle
xdotool type --delay 60 "E2EWIN1"
xdotool key --clearmodifiers ctrl+shift+n
SEC_WID=$(wait_win 'terminator-rust #') || { tail "$ROOT/app.log"; fail "second window not found"; }
echo "root=$ROOT_WID secondary=$SEC_WID"
until xdotool windowfocus "$SEC_WID" 2>/dev/null; do sleep 0.3; done
sleep 1

ids=$(pane_ids)
[ "$(echo "$ids" | wc -l)" -eq 2 ] || { "$CTL" list; fail "expected 2 panes, got: $ids"; }
P_ROOT=$(echo "$ids" | head -1)
P_SEC=$(echo "$ids" | tail -1)
[ "$P_ROOT" != "$P_SEC" ] || fail "pane ids collide across windows"
wait_capture "$P_ROOT" E2EWIN1 || { "$CTL" capture "$P_ROOT" | tail -3; fail "root pane missing marker"; }

# --- W2: typing goes to the focused window only ---------------------------
step "W2: typing lands in the secondary pane only"
xdotool type --delay 60 "E2EWIN2"
wait_capture "$P_SEC" E2EWIN2 || { "$CTL" capture "$P_SEC" | tail -3; fail "secondary pane missing marker"; }
if "$CTL" capture "$P_SEC" 2>/dev/null | grep -q E2EWIN1; then
    fail "secondary pane echoed the ROOT marker - panes are not independent"
fi
if "$CTL" capture "$P_ROOT" 2>/dev/null | grep -q E2EWIN2; then
    fail "root pane echoed the SECONDARY marker - keyboard leaked cross-window"
fi

# --- W3: closing the secondary's last pane removes that window -----------
step "W3: Ctrl+Shift+W in the secondary kills only that window"
xdotool key --clearmodifiers ctrl+shift+w
wait_win_gone 'terminator-rust #' || fail "secondary window survived its last pane close"
kill -0 "$APP_PID" 2>/dev/null || fail "app died with the secondary window"
wait_win '^terminator-rust$' >/dev/null || fail "root window vanished"
[ "$(pane_ids | wc -l)" -eq 1 ] || { "$CTL" list; fail "pane bookkeeping after window removal"; }
echo "app alive, root window intact, panes=1"

# --- W4: re-spawn works, window ids progress ------------------------------
step "W4: second Ctrl+Shift+N re-spawns a working window"
xdotool windowfocus "$ROOT_WID"
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done
xdotool key --clearmodifiers ctrl+shift+n
SEC2_WID=$(wait_win 'terminator-rust #') || fail "re-spawned window not found"
[ "$SEC2_WID" != "$SEC_WID" ] || fail "re-spawned window reused the old X id"
until xdotool windowfocus "$SEC2_WID" 2>/dev/null; do sleep 0.3; done
sleep 1
ids=$(pane_ids)
[ "$(echo "$ids" | wc -l)" -eq 2 ] || { "$CTL" list; fail "expected 2 panes after re-spawn"; }
P_SEC2=$(echo "$ids" | tail -1)
xdotool type --delay 60 "E2EWIN3"
wait_capture "$P_SEC2" E2EWIN3 || fail "re-spawned pane missing marker"

# --- W6: root's last pane close respawns a tab (sibling alive) ------------
step "W6: closing the ROOT window's last pane respawns a fresh root tab"
xdotool windowfocus "$ROOT_WID"
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done
sleep 1
xdotool key --clearmodifiers ctrl+shift+w
kill -0 "$APP_PID" 2>/dev/null || { tail "$ROOT/app.log"; fail "app quit on root last-pane close with a sibling alive"; }
wait_win '^terminator-rust$' >/dev/null || fail "root window vanished with a sibling alive"
# Root respawns a NEW pane (fresh id, not the closed one); wait for the
# bookkeeping to settle at 2 panes again.
NEW_ROOT=""
for _ in $(seq 1 40); do
    ids=$(pane_ids)
    if [ "$(echo "$ids" | wc -l)" -eq 2 ] && [ "$ids" != "$P_ROOT $P_SEC2" ]; then
        NEW_ROOT=$(echo "$ids" | grep -vx "$P_SEC2" || true)
        [ -n "$NEW_ROOT" ] && [ "$NEW_ROOT" != "$P_ROOT" ] && break
    fi
    sleep 0.25
done
[ -n "${NEW_ROOT:-}" ] && [ "$NEW_ROOT" != "$P_ROOT" ]     || { "$CTL" list; fail "root did not respawn a fresh pane after its last close"; }
xdotool type --delay 60 "E2EWIN4"
wait_capture "$NEW_ROOT" E2EWIN4 || { "$CTL" capture "$NEW_ROOT" | tail -3; fail "respawned root pane missing marker"; }
wait_capture "$P_SEC2" E2EWIN3 || fail "sibling pane disturbed by root respawn"
echo "root respawned pane $NEW_ROOT (was $P_ROOT), sibling intact"

# --- W7: Ctrl+Shift+J hands the secondary's tab back to the root ----------
step "W7: Ctrl+Shift+J from the secondary moves its tab into the root"
SEC_PID=$(pane_pid "$P_SEC2")
[ -n "$SEC_PID" ] || { "$CTL" list; fail "no child pid for pane $P_SEC2"; }
xdotool windowfocus "$SEC2_WID"
until xdotool windowfocus "$SEC2_WID" 2>/dev/null; do sleep 0.3; done
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+j
wait_win_gone 'terminator-rust #' || fail "emptied secondary survived Ctrl+Shift+J"
kill -0 "$APP_PID" 2>/dev/null || { tail "$ROOT/app.log"; fail "app died moving the tab"; }
[ "$(pane_ids | wc -l)" -eq 2 ] || { "$CTL" list; fail "pane count changed in W7"; }
[ "$(pane_pid "$P_SEC2")" = "$SEC_PID" ] || { "$CTL" list; fail "moved pane lost its child in W7"; }
# The moved tab arrives ACTIVE in the root: typing must land in it.
xdotool windowfocus "$ROOT_WID"
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done
xdotool type --delay 60 "E2EWIN5"
wait_capture "$P_SEC2" E2EWIN5 || { "$CTL" capture "$P_SEC2" | tail -3; fail "moved pane is not the root's active tab"; }
echo "pane $P_SEC2 now lives in the root (pid $SEC_PID)"

# --- W8: Ctrl+Shift+J with a single window spawns one for the tab ---------
step "W8: Ctrl+Shift+J in the only window spawns a window for the active tab"
ROOT_PID=$(pane_pid "$NEW_ROOT")
[ -n "$ROOT_PID" ] || { "$CTL" list; fail "no child pid for pane $NEW_ROOT"; }
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+j
SEC3_WID=$(wait_win 'terminator-rust #') || { tail "$ROOT/app.log"; fail "Ctrl+Shift+J spawned no window"; }
kill -0 "$APP_PID" 2>/dev/null || { tail "$ROOT/app.log"; fail "app died in W8"; }
[ "$(pane_ids | wc -l)" -eq 2 ] || { "$CTL" list; fail "W8 leaked the seed tab's pane"; }
[ "$(pane_pid "$P_SEC2")" = "$SEC_PID" ] || fail "W8 disturbed the moved pane"
[ "$(pane_pid "$NEW_ROOT")" = "$ROOT_PID" ] || fail "W8 disturbed the root pane"
until xdotool windowfocus "$SEC3_WID" 2>/dev/null; do sleep 0.3; done
xdotool type --delay 60 "E2EWIN6"
wait_capture "$P_SEC2" E2EWIN6 || fail "the spawned window does not own the moved pane"
xdotool windowfocus "$ROOT_WID"
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done
xdotool type --delay 60 "E2EWIN7"
wait_capture "$NEW_ROOT" E2EWIN7 || fail "the root lost its own tab in W8"
echo "window split: $NEW_ROOT in the root, $P_SEC2 in $SEC3_WID"

# --- W9: Ctrl+Shift+M merges every window back into the root --------------
step "W9: Ctrl+Shift+M merges the windows"
xdotool windowfocus "$SEC3_WID"
until xdotool windowfocus "$SEC3_WID" 2>/dev/null; do sleep 0.3; done
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+m
wait_win_gone 'terminator-rust #' || fail "merge left a secondary window alive"
kill -0 "$APP_PID" 2>/dev/null || { tail "$ROOT/app.log"; fail "app died in the merge"; }
[ "$(pane_ids | wc -l)" -eq 2 ] || { "$CTL" list; fail "merge lost panes"; }
[ "$(pane_pid "$P_SEC2")" = "$SEC_PID" ] || fail "merge disturbed the moved pane"
[ "$(pane_pid "$NEW_ROOT")" = "$ROOT_PID" ] || fail "merge disturbed the root pane"
xdotool windowfocus "$ROOT_WID"
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done
wait_capture "$NEW_ROOT" E2EWIN7 || fail "merged root lost its tab's screen"
echo "merged back: panes $NEW_ROOT + $P_SEC2 in the root"

# --- W10: cross-window chip drag hands the tab to another window ----------
step "W10: dragging the chip onto the other window's strip moves the tab"
xdotool windowfocus "$ROOT_WID"
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+n
DRAG_WID=$(wait_win 'terminator-rust #') || { tail "$ROOT/app.log"; fail "no secondary for the drag check"; }
until xdotool windowfocus "$DRAG_WID" 2>/dev/null; do sleep 0.3; done
sleep 1
DRAG_PANE=$(pane_ids | tail -1)
DRAG_PID=$(pane_pid "$DRAG_PANE")
[ -n "$DRAG_PID" ] || { "$CTL" list; fail "no child pid for the drag pane"; }
# Bare Xvfb has no WM: the newest window sits on top, but raise the source
# explicitly - the press must hit ITS chip, not the root's chrome below.
xdotool windowraise "$DRAG_WID"
read X Y DRAG_W DRAG_H <<<"$(win_geom "$DRAG_WID")"
read RX RY ROOT_W ROOT_H <<<"$(win_geom "$ROOT_WID")"
# Park the pointer inside the source's pane first: bare chrome there is
# live (hover fills / icon cells) and would skew the baseline scrot.
xdotool mousemove --sync "$(( X + DRAG_W * 2 / 3 ))" "$(( Y + 200 ))"
sleep 0.5
scrot -o "$ROOT/w10-base.png"
CHIP_EDGES=$(chip_edges "$ROOT/w10-base.png" "$X" "$Y" "$DRAG_W") \
    || fail "no active chip found in the source window's row"
read CHIP_L CHIP_R <<<"$CHIP_EDGES"
CHIPX=$(( (CHIP_L + CHIP_R) / 2 ))
CHIPY=$(( Y + 18 ))
DRAGX=$(( RX + ROOT_W / 2 ))
DRAGY=$(( RY + 18 ))
BAND0=$(( RX + 400 ))              # bare chrome right of any chip...
BAND1=$(( RX + ROOT_W - 200 ))     # ...and left of the fixed cells
# The press frame must land on the chip BEFORE the first motion frame
# (egui hit-tests per frame: a coalesced press+move latches bare chrome
# and turns into a window-move StartDrag instead).
xdotool mousemove --sync "$CHIPX" "$CHIPY"
sleep 0.4
xdotool mousedown 1
sleep 0.4
xdotool mousemove --sync "$CHIPX" "$(( (CHIPY + DRAGY) / 2 ))"
sleep 0.2
xdotool mousemove --sync "$DRAGX" "$DRAGY"
sleep 0.6
# Hovering the foreign strip tints it (chips paint over their own slice,
# so the assertion band is bare chrome): the band must change.
scrot -o "$ROOT/w10-over.png"
TINT=$(strip_diff "$ROOT/w10-base.png" "$ROOT/w10-over.png" "$BAND0" "$BAND1" "$RY")
[ "$TINT" -ge 5000 ] || fail "no drop tint on the target strip while hovering (${TINT}px changed)"
xdotool mouseup 1
sleep 0.5
# The source window held only this tab: it is gone, the app is not.
wait_win_gone 'terminator-rust #' || { tail "$ROOT/app.log"; fail "cross-window drop left the source window alive"; }
kill -0 "$APP_PID" 2>/dev/null || { tail "$ROOT/app.log"; fail "app died in the cross-window drop"; }
[ "$(pane_ids | wc -l)" -eq 3 ] || { "$CTL" list; fail "cross-window drop lost panes"; }
[ "$(pane_pid "$DRAG_PANE")" = "$DRAG_PID" ] || { "$CTL" list; fail "moved pane lost its child in W10"; }
[ "$(pane_win "$DRAG_PANE")" = "1" ] || { "$CTL" list; fail "moved pane did not land in the root window"; }
scrot -o "$ROOT/w10-after.png"
AFTER=$(strip_diff "$ROOT/w10-base.png" "$ROOT/w10-after.png" "$BAND0" "$BAND1" "$RY")
[ "$AFTER" -le 50 ] || fail "drop tint lingered on the receiving strip (${AFTER}px changed)"
# The dropped tab arrives ACTIVE in the receiving window: typing lands there.
xdotool windowfocus "$ROOT_WID"
until xdotool windowfocus "$ROOT_WID" 2>/dev/null; do sleep 0.3; done
xdotool type --delay 60 "E2EWIN8"
wait_capture "$DRAG_PANE" E2EWIN8 || { "$CTL" capture "$DRAG_PANE" | tail -3; fail "dropped tab is not the receiver's active tab"; }
echo "dragged pane $DRAG_PANE (pid $DRAG_PID) into the root"

# --- W5: Ctrl+Shift+Q from a secondary quits the whole app ----------------
step "W5: Ctrl+Shift+Q in the secondary quits the app"
xdotool key --clearmodifiers ctrl+shift+n
QUIT_WID=$(wait_win 'terminator-rust #') || { tail "$ROOT/app.log"; fail "no secondary for the quit check"; }
until xdotool windowfocus "$QUIT_WID" 2>/dev/null; do sleep 0.3; done
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+q
wait_pid_gone "$APP_PID" 40 || { tail "$ROOT/app.log"; fail "app survived Ctrl+Shift+Q from secondary"; }
echo "app exited cleanly"

step "PASS: multi-window e2e"
