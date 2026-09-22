#!/usr/bin/env bash
# Headless e2e: empty-window state restore + the exit -> quit -> restore
# loop (regression: "opens to nothing" after every shell exited).
#   E1: a state.json holding `windows:[{tabs:[]}]` (what the quit path
#       writes when the last shell exits) restores as a LIVE tab: ctl
#       list is non-empty and the respawned shell echoes a marker.
#   E2: `exit` in the only pane auto-closes it (EXIT_GRACE), the app
#       quits, and the persisted state is the empty-window shape again.
#   E3: relaunching from that state restores live again (E1 loop).
#
# Usage: scripts/bin/e2e-empty-restore.sh  (repo root; Xvfb + xdotool).
#        E2E_KEEP=1 keeps the scratch dir for debugging.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-empty-XXXXXX)
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

cargo build -p app -p ctl --bins >/dev/null

export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1
# e2e presets rely on session restore; the default launch is a fresh tab
export TERMINATOR_RESTORE=1
mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"
export SHELL=/bin/sh
STATE="$HOME/.terminator-rust/state.json"

# The exact shape the quit path writes when the last shell exits.
cat >"$STATE" <<'JSON'
{"theme":"dracula","windows":[{"id":1,"active_tab":0,"tabs":[]}]}
JSON

# --- Xvfb -----------------------------------------------------------------
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

# list_pane_id: echoes the single pane id from `list --json`.
list_pane_id() {
    "$CTL" list --json 2>/dev/null | grep -o '"id": [0-9]*' | grep -o '[0-9]*' | head -1
}

# wait_list [tries]: echoes the pane id once `ctl list` reports one.
wait_list() {
    local tries=${1:-40} id
    for _ in $(seq 1 "$tries"); do
        id=$(list_pane_id)
        if [ -n "$id" ]; then
            echo "$id"
            return 0
        fi
        sleep 0.25
    done
    return 1
}

# wait_capture <pane-id> <marker> [tries]
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

# --- E1: empty state restores as a LIVE tab -------------------------------
step "E1: launch from windows:[{tabs:[]}] -> live tab"
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app1.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app1.log"; fail "control socket never appeared"; }
P1=$(wait_list) || { tail "$ROOT/app1.log"; "$CTL" list; \
    fail "restore from empty tabs respawned NOTHING (the opens-to-nothing bug)"; }
echo "respawned pane $P1"
"$CTL" send "$P1" --text "echo E2E_RESTORE_OK"$'\n' >/dev/null
wait_capture "$P1" E2E_RESTORE_OK || { "$CTL" capture "$P1" | tail -3; \
    fail "respawned pane has no live shell (marker never echoed)"; }

# --- E2: exit -> auto-close -> app quits, state is empty again ------------
step "E2: exit auto-closes the pane and quits the app"
"$CTL" send "$P1" --text "exit"$'\n' >/dev/null
for _ in $(seq 1 60); do
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.25
done
if kill -0 "$APP_PID" 2>/dev/null; then
    "$CTL" list || true
    fail "app still alive 15s after the last shell exited (auto-close/quit broken)"
fi
APP_PID=""
grep -q '"tabs": *\[\]' "$STATE" || { python3 -m json.tool "$STATE"; \
    fail "quit path did not persist the empty-window state"; }
echo "app quit, state persisted empty tabs"

# --- E3: relaunch from the freshly-written empty state --------------------
step "E3: relaunch from the just-written empty state"
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app2.log" 2>&1 & APP_PID=$!
P2=$(wait_list) || { tail "$ROOT/app2.log"; fail "second restore respawned nothing"; }
"$CTL" send "$P2" --text "echo E2E_SECOND_OK"$'\n' >/dev/null
wait_capture "$P2" E2E_SECOND_OK || { "$CTL" capture "$P2" | tail -3; \
    fail "second respawn has no live shell"; }
kill "$APP_PID" 2>/dev/null || true
APP_PID=""
echo "PASS: empty-window restore + exit/quit/restore loop all green"
