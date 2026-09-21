#!/usr/bin/env bash
# Headless e2e: window controls under a real EWMH WM (openbox on Xvfb).
#   R1: edge drags resize the window APP-DRIVEN: per-frame
#       ViewportCommands steered by an X-polled pointer (the WM only
#       applies geometry). R1a: an east drag lands the exact pointer
#       width. R1b: after release the geometry is FROZEN - pointer
#       wander must not follow (the original bug). R1c: a west drag
#       moves the origin, the direction whose per-frame XMoveWindow
#       used to break the WM pointer grab.
#   R2: the chrome maximize/restore button toggles ViewportCommand::
#       Maximized (openbox has no panel -> maximize == 1400x900) and the
#       second press restores the pre-maximize size.
#   R3: double-clicking BARE chrome (between the chips and the fixed
#       right-hand buttons) toggles maximize / restore the same way.
#   R4: the minimize button iconifies the window: no longer
#       --onlyvisible while the app stays alive; windowactivate restores.
#   R5: Ctrl+Shift+Q quits the app cleanly.
#   S1: at scroll 0 (wheel-up normalised) the chip slot at
#       (chip-right - 40px, safely left of the per-chip close button)
#       holds an EARLY tab, not the last one.
#   S2: wheel notches (48px each) scroll the chip row to its end so the
#       LAST tab lands in that slot; no tab is lost on the way.
#   S3: with the chip row overflowing, the trailing '+' button stays at
#       its FIXED right-edge slot and still spawns + activates tab 13.
#   S4: Ctrl+Shift+Q quits the app cleanly.
#
# Chrome geometry contract under test (single ~36px row): fixed
# right-hand buttons, 16px wide / 4px apart / rightmost 5px from the
# edge - maximize center at W-13, minimize W-33, inspector W-53, zoom
# W-73; the trailing group ('+', split-v, split-h) starts 8px further
# left, first '+' center at W-137; the chip area ends at W-153. The
# chip-row center y sits at Y+18. Vertical wheel = horizontal chip
# scroll, one notch = 48px.
#
# Usage: scripts/bin/e2e-window-controls.sh  (repo root; needs Xvfb +
#        xdotool + openbox).  E2E_KEEP=1 keeps the scratch dir.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-wc-XXXXXX)
APP=target/debug/terminator-rust
DISPLAY_N=""                  # probed below (stale sockets break ":$$")
XVFB_PID=""
OPENBOX_PID=""
APP_PID=""
WID=""
CHROME_Y=18                   # chip-row center y offset inside the window

cleanup() {
    [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
    [ -n "$OPENBOX_PID" ] && kill "$OPENBOX_PID" 2>/dev/null || true
    [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
    sleep 0.3
    if [ "${E2E_KEEP:-0}" = "1" ]; then
        echo "(E2E_KEEP=1: scratch dir kept at $ROOT)"
    else
        rm -rf "$ROOT"
    fi
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; tail -20 "$ROOT"/app*.log 2>/dev/null || true; exit 1; }
step() { echo "== $*"; }

# --- window helpers -------------------------------------------------------

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

wait_pid_gone() {
    local pid=$1 tries=${2:-40}
    for _ in $(seq 1 "$tries"); do
        kill -0 "$pid" 2>/dev/null || return 0
        sleep 0.25
    done
    return 1
}

# geo: refresh the X/Y/WIDTH/HEIGHT shell vars from the live window.
geo() {
    local out
    out=$(xdotool getwindowgeometry --shell "$WID") \
        || fail "getwindowgeometry failed for '$WID'"
    eval "$out"
}

# wait_geo <desc> <predicate-fn>: poll the predicate against fresh
# geometry every 0.25s (40 tries = 10s), then re-check once after the
# final sleep so a last-instant WM transition is not missed.
wait_geo() {
    local desc=$1
    shift
    for _ in $(seq 1 40); do
        geo
        if "$@"; then return 0; fi
        sleep 0.25
    done
    geo
    if "$@"; then return 0; fi
    fail "geometry never became: $desc (now ${X},${Y} ${WIDTH}x${HEIGHT})"
}

is_maximized()     { [ "$WIDTH" -eq 1400 ] && [ "$HEIGHT" -eq 900 ]; }
is_not_maximized() { [ "$WIDTH" -lt 1400 ]; }

# near_target: un-maximized and within +/-10px of $W_TARGET (the width
# captured right after the R1 resize).
near_target() {
    is_not_maximized || return 1
    local d=$((WIDTH - W_TARGET))
    [ "$d" -le 10 ] && [ "$((0 - d))" -le 10 ]
}

# park <x> <y>: move the pointer and let an app frame hit-test it
# (egui hit-tests per frame at a ~50ms cadence; an instant click races
# it and can land on the previous frame's target).
park() { xdotool mousemove "$1" "$2"; sleep 0.3; }

# click_at <x> <y>: park, then a global XTest button-1 click (never
# --window: winit drops XSendEvent-delivered input).
click_at() { park "$1" "$2"; xdotool click 1; sleep 0.5; }

# activate: EWMH-activate the window. With a WM present, windowfocus
# alone does NOT deliver keyboard events; retry while the WM settles.
activate() {
    for _ in $(seq 1 20); do
        xdotool windowactivate "$WID" 2>/dev/null && return 0
        sleep 0.3
    done
    fail "windowactivate never succeeded for $WID"
}

# --- state.json helpers ---------------------------------------------------

tab_count() {
    python3 -c 'import json,sys
print(len(json.load(open(sys.argv[1]))["windows"][0]["tabs"]))' "$STATE"
}

active_tab() {
    python3 -c 'import json,sys
print(json.load(open(sys.argv[1]))["windows"][0]["active_tab"])' "$STATE"
}

# launch_app <log>: start the app on the running Xvfb, wait for the
# control socket + window, EWMH-activate it, snapshot the geometry.
launch_app() {
    local log=$1
    rm -f "$SOCK"    # start() reclaims stale sockets; keep the wait honest
    env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid nohup "$APP" </dev/null \
        >"$ROOT/$log" 2>&1 & APP_PID=$!
    for _ in $(seq 1 40); do
        kill -0 "$APP_PID" 2>/dev/null \
            || { tail "$ROOT/$log"; fail "app died at startup"; }
        [ -S "$SOCK" ] && break
        sleep 0.25
    done
    [ -S "$SOCK" ] || { tail "$ROOT/$log"; fail "control socket never appeared"; }
    WID=$(wait_win '^terminator-rust$') \
        || { tail "$ROOT/$log"; fail "app window not found"; }
    activate
    sleep 1
    geo
    kill -0 "$APP_PID" 2>/dev/null || fail "app died during startup"
    echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"
}

# --- build + sandbox ------------------------------------------------------
cargo build -p app -p ctl --bins >/dev/null

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/sh      # no OSC title churn; window names stay stable
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
STATE="$XDG_CONFIG_HOME/terminator-rust/state.json"
export TERMINATOR_OPAQUE=1     # no compositor on Xvfb
export TERMINATOR_NO_MOTION=1  # pin hover fades / cursor blink
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# --- Xvfb + openbox -------------------------------------------------------
step "launch Xvfb + openbox"
# Random display probe: a fixed/PID-derived number can collide with a
# stale /tmp/.X11-unix socket left by an earlier crashed run.
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
export DISPLAY="$DISPLAY_N"   # for xdotool (app gets it via env below)

# A real EWMH WM: keyboard focus needs windowactivate, and maximize /
# minimize / move-resize land via EWMH properties.
env DISPLAY="$DISPLAY_N" setsid openbox >/dev/null 2>&1 & OPENBOX_PID=$!
sleep 1
kill -0 "$OPENBOX_PID" 2>/dev/null || fail "openbox failed to start"

# --- part 1: window controls (fresh state, single "shell" tab) ------------
step "part 1: window controls on a fresh state"
launch_app app1.log

step "R1a: right-edge drag resizes to the exact pointer width"
geo
W0=$WIDTH; X0=$X
park $((X+WIDTH-2)) $((Y+HEIGHT/2))     # inside the 6px edge zone
xdotool mousedown 1
sleep 0.3     # press and first move must not share a frame (anchor race)
for i in 1 2 3 4 5 6 7 8; do
    xdotool mousemove $((X0+W0-2+i*10)) $((Y+HEIGHT/2)); sleep 0.06
done
xdotool mouseup 1
sleep 0.6
geo
[ "$WIDTH" -eq $((W0+80)) ] \
    || fail "edge drag: ${W0} -> ${WIDTH} (want exactly ${W0}+80)"
echo "R1a: width ${W0} -> ${WIDTH} exactly"

step "R1b: after release the window stops following the pointer"
for px in $((X0+40)) $((X0+W0-2)) $((X0+W0+40)) $((X0+WIDTH/2)); do
    xdotool mousemove "$px" $((Y+HEIGHT/2)); sleep 0.3
done
geo
[ "$WIDTH" -eq $((W0+80)) ] && [ "$X" -eq "$X0" ] \
    || fail "post-release follow: ${X},${Y} ${WIDTH}x${HEIGHT}"
echo "R1b: frozen at ${X},${Y} ${WIDTH}x${HEIGHT}"

step "R1c: west-edge drag moves the origin, width grows"
park $X0 $((Y+HEIGHT/2))                # first pixel column = west strip
xdotool mousedown 1
sleep 0.3
for i in 1 2 3 4 5 6; do
    xdotool mousemove $((X0-i*10)) $((Y+HEIGHT/2)); sleep 0.06
done
xdotool mouseup 1
sleep 0.6
geo
[ "$X" -eq $((X0-60)) ] && [ "$WIDTH" -eq $((W0+140)) ] \
    || fail "west drag: ${X},${WIDTH} (want $((X0-60)),$((W0+140)))"
echo "R1c: origin ${X0} -> ${X}, width -> ${WIDTH}"
W_TARGET=$WIDTH

step "R2: maximize button toggles 1400x900 and restores"
click_at $((X+WIDTH-13)) $((Y+CHROME_Y))    # maximize/restore
wait_geo "maximized (1400x900)" is_maximized
click_at $((X+WIDTH-13)) $((Y+CHROME_Y))
wait_geo "restored to ~${W_TARGET}px" near_target
echo "R2: toggled to 1400x900 and back to ${WIDTH}"

step "R3: double-click on bare chrome toggles maximize/restore"
park $((X+WIDTH*60/100)) $((Y+CHROME_Y))    # bare chrome right of the chip
xdotool click --repeat 2 --delay 150 1
sleep 0.5
wait_geo "maximized via double-click" is_maximized
park $((X+WIDTH*60/100)) $((Y+CHROME_Y))
xdotool click --repeat 2 --delay 150 1
sleep 0.5
wait_geo "restored via double-click" is_not_maximized
echo "R3: double-click toggle ok (${WIDTH}x${HEIGHT})"

step "R4: minimize button iconifies; windowactivate restores"
click_at $((X+WIDTH-33)) $((Y+CHROME_Y))    # minimize
sleep 0.5
ids=$(xdotool search --onlyvisible --name '^terminator-rust$' 2>/dev/null || true)
[ -z "$ids" ] || fail "window still visible after minimize: $ids"
kill -0 "$APP_PID" 2>/dev/null || fail "app died on minimize"
echo "R4: minimized (iconic, app alive)"
activate
vis=""
for _ in $(seq 1 40); do
    ids=$(xdotool search --onlyvisible --name '^terminator-rust$' 2>/dev/null || true)
    if [ -n "$ids" ]; then vis=1; break; fi
    sleep 0.25
done
[ -n "$vis" ] || fail "window never became visible again after windowactivate"
geo
echo "R4: restored at ${X},${Y} ${WIDTH}x${HEIGHT}"

step "R5: Ctrl+Shift+Q quits the app"
activate
xdotool key --clearmodifiers ctrl+shift+q
wait_pid_gone "$APP_PID" 40 || fail "app survived Ctrl+Shift+Q (part 1)"
echo "app exited cleanly"
APP_PID=""
WID=""
sleep 0.5

# --- part 2: chip-row overflow scrolling ----------------------------------
step "part 2: preset 12 overflowing tabs and relaunch"
python3 - "$STATE" <<'PY'
import json, sys
# 12 single-pane tabs "tabname-01".."tabname-12": fixed-width titles ->
# uniform ~106px chips, 12 * 106 = 1272px > the ~1047px chip area of a
# 1200px window. Pane/meta shape mirrors e2e-dragdrop.sh's preset (ids
# are remapped in preorder on load anyway).
meta = {"kind": "Local", "manual_title": None, "bg": None,
        "transparency": 0.0, "degraded": False}
tabs = []
for i in range(1, 13):
    pid = 10 + i
    tabs.append({"title": "tabname-%02d" % i, "focused": pid,
                 "root": {"Pane": {"id": pid, "meta": meta}}})
state = {"theme": "dracula",
         "settings": {"split_axis": "v", "split_ratio": 0.5},
         "windows": [{"id": 1, "active_tab": 0, "tabs": tabs}]}
with open(sys.argv[1], "w") as f:
    json.dump(state, f, indent=2)
PY

launch_app app2.log
sleep 1   # let the 12 preset shells settle
geo
[ "$WIDTH" -eq 1200 ] && [ "$HEIGHT" -eq 800 ] \
    || fail "expected the default 1200x800 window, got ${WIDTH}x${HEIGHT}"

# 12 preset tabs: the LAST one is index 11. Normalise the strip to scroll 0
# first (the auto-follow could have scrolled it into view at preset time).
step "S1: at scroll 0, the chip at right-60 is an EARLY tab"
park $((X+WIDTH/2)) $((Y+CHROME_Y))
for _ in $(seq 1 10); do xdotool click 4; sleep 0.12; done   # wheel up = scroll 0
sleep 0.5
click_at $((X+WIDTH-153-40)) $((Y+CHROME_Y))
A=""
for _ in $(seq 1 20); do
    A=$(active_tab)
    [ "$A" != "11" ] && break
    sleep 0.25
done
[ "$A" != "11" ] \
    || fail "right-40 slot already held the LAST tab at scroll 0"
[ "$(tab_count)" -eq 12 ] \
    || fail "the S1 click hit a close button (tabs=$(tab_count))"
echo "S1: slot holds tab index ${A}"

step "S2: 10 wheel notches (48px each) scroll the chip row to its end"
park $((X+WIDTH/2)) $((Y+CHROME_Y))
for _ in $(seq 1 10); do xdotool click 5; sleep 0.12; done
sleep 0.5
click_at $((X+WIDTH-153-40)) $((Y+CHROME_Y))
ok=""
for _ in $(seq 1 20); do
    if [ "$(active_tab)" -eq 11 ]; then ok=1; break; fi
    sleep 0.25
done
[ -n "$ok" ] \
    || fail "scroll did not bring the LAST tab into the right-40 slot (active=$(active_tab))"
[ "$(tab_count)" -eq 12 ] || fail "scrolling lost tabs: count=$(tab_count)"
echo "S2: last tab now under the slot; 12 tabs intact"

step "S3: overflow does not push the trailing '+' off its fixed slot"
click_at $((X+WIDTH-137)) $((Y+CHROME_Y))   # '+' (new tab)
ok=""
for _ in $(seq 1 20); do
    if [ "$(tab_count)" -eq 13 ] && [ "$(active_tab)" -eq 12 ]; then ok=1; break; fi
    sleep 0.25
done
[ -n "$ok" ] \
    || fail "'+' did not add+activate tab 13 (tabs=$(tab_count) active=$(active_tab))"
echo "S3: 13 tabs, active_tab=12"

step "S4: Ctrl+Shift+Q quits the app"
activate
xdotool key --clearmodifiers ctrl+shift+q
wait_pid_gone "$APP_PID" 40 || fail "app survived Ctrl+Shift+Q (part 2)"
echo "app exited cleanly"

echo "WINDOW-CONTROLS E2E: PASS"
