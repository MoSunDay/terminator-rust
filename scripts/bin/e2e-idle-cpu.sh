#!/usr/bin/env bash
# e2e-idle-cpu.sh - reactive render loop regression gate.
#
# The app must NOT run a repaint tick: idle windows cost (almost) nothing
# and PTY output / IPC requests wake the loop on demand. This script
# measures the app process CPU (utime+stime from /proc/<pid>/stat) over:
#   1. an IDLE window (cursor blink edges only) - asserted against a
#      generous ceiling that the old unconditional 16/50ms tick blows
#      through (it kept every frame rendering at 20-60fps forever);
#   2. a WAKE roundtrip through a fully-idle app: `ctl send` writes to a
#      pane, the echoed marker must show up in `ctl capture` quickly -
#      with no tick left, this only works if the pty-reader wake and the
#      IPC listener wake actually request repaints;
#   3. a BUSY window while `cat` streams a large file - informational
#      (printed, not asserted): output-driven frames are supposed to
#      burn CPU while there is output.
#
# Usage: scripts/bin/e2e-idle-cpu.sh   (from the repo root; needs Xvfb +
# xdotool + python3). Set E2E_KEEP=1 to keep the scratch dir.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-cpu-XXXXXX)
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

# Build first: the sandboxed HOME below would hide rustup/toolchains.
cargo build -p app -p ctl --bins >/dev/null

export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/bash
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
# Opaque for pixel determinism is irrelevant here, but pin it anyway to
# match the other e2e scripts. NO_MOTION is deliberately NOT set: the
# cursor blink must run its real (square-wave) cadence so the idle
# measurement includes the blink-edge wakeups it costs.
export TERMINATOR_OPAQUE=1
mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# --- Xvfb + app ----------------------------------------------------------
step "launch Xvfb + app"
DISPLAY_N=""
for _ in $(seq 1 12); do
    N=$((100 + RANDOM % 880))
    [ -S "/tmp/.X11-unix/X$N" ] && continue
    Xvfb ":$N" -screen 0 1200x800x24 & XVFB_PID=$!
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
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "ipc socket never appeared"; }
sleep 2   # let the shell prompt settle

# setsid execs into the app (the background child is no session leader),
# so $! is the app pid; pgrep is only a fallback for exotic shells.
[ -r "/proc/$APP_PID/stat" ] || APP_PID=$(pgrep -x terminator-rust | tail -1)
[ -r "/proc/$APP_PID/stat" ] || fail "no /proc entry for app pid $APP_PID"

# cpu_ticks: utime+stime of the app process, in clock ticks (usually 100/s).
cpu_ticks() { awk '{print $14+$15}' "/proc/$APP_PID/stat"; }

# --- 1. idle CPU ----------------------------------------------------------
step "idle CPU over 5s (blink edges only)"
IDLE_SECS=5
# Settle one blink period first so a settle frame does not pollute it.
sleep 2
t0=$(cpu_ticks); sleep "$IDLE_SECS"; t1=$(cpu_ticks)
idle=$((t1 - t0))
HZ=$(getconf CLK_TCK)
pct=$(IDLE=$idle HZ=$HZ SECS=$IDLE_SECS python3 -c \
    'import os; print(f"{int(os.environ["IDLE"])*100/int(os.environ["HZ"])/float(os.environ["SECS"]):.1f}")')
echo "idle: ${idle} ticks over ${IDLE_SECS}s (~${pct}% of one core)"
# Ceiling: 100 ticks = 1.0s CPU over the 5s window (20% of a core). The
# old 20fps-always tick rendered ~300+ ticks even fully idle.
MAX_IDLE_TICKS=100
[ "$idle" -le "$MAX_IDLE_TICKS" ] \
    || fail "idle burned $idle ticks (> $MAX_IDLE_TICKS): the reactive loop regressed to a tick"

# --- 2. wake roundtrip through an idle app --------------------------------
step "wake roundtrip (pty + ipc) through the idle app"
PANE=$("$CTL" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')
MARKER="WAKE_OK_$RANDOM"
"$CTL" send "$PANE" --text "echo $MARKER" >/dev/null
seen=""
START=$(date +%s%N)
for _ in $(seq 1 40); do
    if "$CTL" capture "$PANE" 2>/dev/null | grep -qF -- "$MARKER"; then
        seen=yes
        break
    fi
    sleep 0.05
done
END=$(date +%s%N)
[ -n "$seen" ] || { "$CTL" capture "$PANE"; fail "marker never appeared: pty/ipc wake is broken"; }
ms=$(( (END - START) / 1000000 ))
echo "wake roundtrip: marker visible after ~${ms}ms (needs no repaint tick)"
[ "$ms" -le 2000 ] || fail "wake took ${ms}ms - output only arrived via a fallback path"

# --- 3. busy output (informational) ---------------------------------------
step "busy CPU while cat streams (informational)"
python3 - "$ROOT/big.txt" <<'PY'
import sys
with open(sys.argv[1], "w") as f:
    for i in range(40000):
        f.write(f"line {i} the quick brown fox jumps over the lazy dog\n")
PY
BUSY_SECS=3
"$CTL" send "$PANE" --text "cat $ROOT/big.txt" >/dev/null
b0=$(cpu_ticks); sleep "$BUSY_SECS"; b1=$(cpu_ticks)
busy=$((b1 - b0))
echo "busy: ${busy} ticks over ${BUSY_SECS}s (output-driven frames; expected >> idle)"

step "PASS: idle=${idle} ticks, wake=${ms}ms, busy=${busy} ticks"
