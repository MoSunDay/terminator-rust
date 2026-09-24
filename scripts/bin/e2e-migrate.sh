#!/usr/bin/env bash
# Headless e2e: cross-process tab migration over the control sockets.
# Two app instances run side by side; M1 moves instance A's only pane into
# instance B over a real UDS + SCM_RIGHTS hand-over.
#   M1: `ctl migrate` reports 1 pane moved and B now lists that pane with
#       the SAME child pid and the screen the pane had in A (snapshot).
#   M2: the pane is genuinely alive in B: typing into it echoes back.
#   M3: A lost the pane but not the app - its emptied root window respawns
#       a fresh shell (new pid), proving the tab removal never killed it.
#   M4: the migrated child process is STILL RUNNING (same pid, kill -0).
#   M5: migrating it back to A works (round trip, state carried again).
#
# Usage: scripts/bin/e2e-migrate.sh   (repo root; needs Xvfb).
#        E2E_KEEP=1 keeps the scratch dir for debugging.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-migrate-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
XVFB_PID=""
APP_A=""
APP_B=""

cleanup() {
    [ -n "$APP_A" ] && kill "$APP_A" 2>/dev/null || true
    [ -n "$APP_B" ] && kill "$APP_B" 2>/dev/null || true
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

# The pretty-printed pane JSON keeps one field per line, so awk tracks the
# id/pid/alive of each pane object.

# pane_pid <socket> <pane-id> -> child pid (empty when the pane is gone)
pane_pid() {
    "$CTL" --socket "$1" list --json 2>/dev/null | awk -v want="$2" '
        /"id": / { id = $2; sub(/,.*/, "", id) }
        /"pid": / { pid = $2; sub(/,.*/, "", pid) }
        /"pid": / && id == want { print pid }
    '
}

# live_ids <socket>: ids of every ALIVE pane.
live_ids() {
    "$CTL" --socket "$1" list --json 2>/dev/null | awk '
        /"id": / { id = $2; sub(/,.*/, "", id) }
        /"alive": true/ { print id }
    ' | sort -n
}

# live_pids <socket>: pids of every ALIVE pane.
live_pids() {
    "$CTL" --socket "$1" list --json 2>/dev/null | awk '
        /"pid": / { pid = $2; sub(/,.*/, "", pid) }
        /"alive": true/ { print pid }
    ' | sort -n
}

# wait_capture <socket> <pane-id> <marker> [tries]
wait_capture() {
    local sock=$1 pane=$2 marker=$3 tries=${4:-40}
    for _ in $(seq 1 "$tries"); do
        if "$CTL" --socket "$sock" capture "$pane" 2>/dev/null | grep -q "$marker"; then
            return 0
        fi
        sleep 0.25
    done
    return 1
}

# one_alive_pane <socket> -> the single alive pane id (fails otherwise)
one_alive_pane() {
    local ids
    ids=$(live_ids "$1")
    [ "$(echo "$ids" | grep -c .)" -eq 1 ] || {
        "$CTL" --socket "$1" list >&2
        fail "expected exactly 1 live pane on $1, got: $ids"
    }
    echo "$ids"
}

cargo build -p app -p ctl --bins >/dev/null

export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1
export SHELL=/bin/sh   # no OSC title churn: list output stays stable
mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR/terminator-rust"
A_SOCK="$XDG_RUNTIME_DIR/terminator-rust/a.sock"
B_SOCK="$XDG_RUNTIME_DIR/terminator-rust/b.sock"

# --- Xvfb + two instances -------------------------------------------------
step "launch Xvfb + two app instances"
DISPLAY_N=""
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
TERMINATOR_SOCK="$A_SOCK" setsid "$APP" >"$ROOT/a.log" 2>&1 & APP_A=$!
TERMINATOR_SOCK="$B_SOCK" setsid "$APP" >"$ROOT/b.log" 2>&1 & APP_B=$!
for _ in $(seq 1 40); do
    [ -S "$A_SOCK" ] && [ -S "$B_SOCK" ] && break
    sleep 0.25
done
[ -S "$A_SOCK" ] && [ -S "$B_SOCK" ] || {
    tail "$ROOT/a.log" "$ROOT/b.log"
    fail "control sockets never appeared"
}

PANE_A=$(one_alive_pane "$A_SOCK")
PANE_B=$(one_alive_pane "$B_SOCK")
PID_A=$(pane_pid "$A_SOCK" "$PANE_A")
echo "A: pane $PANE_A pid $PID_A | B: pane $PANE_B"

# --- M1: hand the pane over ----------------------------------------------
step "M1: migrate A's pane to B"
"$CTL" --socket "$A_SOCK" send "$PANE_A" --text "echo MIG-A1"$'\n' >/dev/null
wait_capture "$A_SOCK" "$PANE_A" MIG-A1 || {
    "$CTL" --socket "$A_SOCK" capture "$PANE_A" | tail -3
    fail "marker never appeared in A"
}
"$CTL" --socket "$A_SOCK" migrate "$PANE_A" --to "$B_SOCK" |
    grep -q "migrated 1 pane" || fail "migrate did not report 1 pane"

# B must list the very same child, on the same pty, with its screen.
MIGRATED=""
for _ in $(seq 1 40); do
    for id in $(live_ids "$B_SOCK"); do
        [ "$(pane_pid "$B_SOCK" "$id")" = "$PID_A" ] && MIGRATED=$id
    done
    [ -n "$MIGRATED" ] && break
    sleep 0.25
done
[ -n "$MIGRATED" ] || {
    "$CTL" --socket "$B_SOCK" list
    fail "B never listed the migrated child (pid $PID_A)"
}
[ "$MIGRATED" != "$PANE_B" ] || fail "migrated pane reused B's pane id"
wait_capture "$B_SOCK" "$MIGRATED" MIG-A1 || {
    "$CTL" --socket "$B_SOCK" capture "$MIGRATED" | tail -3
    fail "adopted pane lost its screen"
}
echo "B: migrated pane $MIGRATED (pid $PID_A)"

# --- M2: the migrated pty is live in B ------------------------------------
step "M2: the adopted pane still answers"
"$CTL" --socket "$B_SOCK" send "$MIGRATED" --text "echo MIG-A2"$'\n' >/dev/null
wait_capture "$B_SOCK" "$MIGRATED" MIG-A2 || fail "typing into the migrated pane did nothing"

# --- M3/M4: A survived, the child was never signalled ---------------------
step "M3/M4: A respawns a shell; the migrated child keeps running"
kill -0 "$PID_A" 2>/dev/null || fail "the migrated child was signalled by the sender"
kill -0 "$APP_A" 2>/dev/null || fail "instance A died with its last tab"
NEW_A=$(one_alive_pane "$A_SOCK")
NEW_PID=$(pane_pid "$A_SOCK" "$NEW_A")
[ "$NEW_PID" != "$PID_A" ] || fail "A's respawned pane reused the migrated child"
[ "$NEW_A" != "$PANE_A" ] || fail "A's respawned pane reused the old pane id"
"$CTL" --socket "$A_SOCK" send "$NEW_A" --text "echo MIG-A3"$'\n' >/dev/null
wait_capture "$A_SOCK" "$NEW_A" MIG-A3 || fail "A's respawned shell is dead"

# --- M5: round trip -------------------------------------------------------
step "M5: migrate the pane back to A"
"$CTL" --socket "$B_SOCK" migrate "$MIGRATED" --to "$A_SOCK" |
    grep -q "migrated 1 pane" || fail "migrate back did not report 1 pane"
BACK=""
for _ in $(seq 1 40); do
    for id in $(live_ids "$A_SOCK"); do
        [ "$(pane_pid "$A_SOCK" "$id")" = "$PID_A" ] && BACK=$id
    done
    [ -n "$BACK" ] && break
    sleep 0.25
done
[ -n "$BACK" ] || {
    "$CTL" --socket "$A_SOCK" list
    fail "the pane did not come back to A"
}
wait_capture "$A_SOCK" "$BACK" MIG-A2 || fail "the round-tripped pane lost its screen"
kill -0 "$PID_A" 2>/dev/null || fail "the round-tripped child died"
if live_pids "$B_SOCK" | grep -q "^$PID_A$"; then
    fail "B still claims the migrated child"
fi
grep -qi "panic" "$ROOT/a.log" "$ROOT/b.log" && fail "a panic landed in an app log"

echo "PASS: migration e2e (M1-M5)"
