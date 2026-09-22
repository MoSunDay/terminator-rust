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

# --- W5: Ctrl+Shift+Q from the secondary quits the whole app --------------
step "W5: Ctrl+Shift+Q in the secondary quits the app"
xdotool windowfocus "$SEC2_WID"
until xdotool windowfocus "$SEC2_WID" 2>/dev/null; do sleep 0.3; done
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+q
wait_pid_gone "$APP_PID" 40 || { tail "$ROOT/app.log"; fail "app survived Ctrl+Shift+Q from secondary"; }
echo "app exited cleanly"

step "PASS: multi-window e2e"
