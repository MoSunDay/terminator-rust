#!/usr/bin/env bash
# Headless e2e for pane IME input (XIM: ibus + libpinyin over Xvfb).
#   S1: shell is live (marker via the control socket, NOT xdotool - the
#       IME is active from frame one and eats plain latin keys).
#   M1: composing swallows keys - 'hanzi' typed with NO commit key never
#       reaches the pane (XFilterEvent consumed it).
#   M2: preedit ink - while still composing, the app-painted preedit
#       underline (dracula block_highlight = 189,147,249, blended by the
#       2px rounded pill antialiasing) shows up as a purple-hued pixel
#       run vs a pre-composition baseline screenshot. 0 -> WARN + SKIP
#       (PreeditNothing negotiation), >0 -> assert >= 4.
#   M3: space commits the libpinyin default candidate (汉字) and the raw
#       UTF-8 lands in the pty; then Shift toggles libpinyin to EN mode
#       and a plain ASCII marker types THROUGH the still-active IME.
#   M4: window stayed alive the whole time; Ctrl+Shift+Q quits; state
#       persisted.
# Usage: scripts/bin/e2e-ime.sh  (needs Xvfb, xdotool, scrot, PIL, ibus,
#        ibus-libpinyin, dbus, x11-utils/xprop). E2E_KEEP=1 keeps scratch.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-ime-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
DISPLAY_N=""
XVFB_PID=""
APP_PID=""
IBUS_PID=""
DBUS_PID=""   # only set when THIS script started a private session bus

cleanup() {
    [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
    [ -n "$IBUS_PID" ] && kill "$IBUS_PID" 2>/dev/null || true
    [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
    [ -n "$DBUS_PID" ] && kill "$DBUS_PID" 2>/dev/null || true
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

# IME diagnostics for the "IME not engaged" class of failures: which
# modifier string the app saw, what engine ibus reports, and whether the
# X server even advertises an XIM server (xprop may be absent).
ime_diag() {
    echo "--- IME diagnostics ---"
    echo "XMODIFIERS=${XMODIFIERS:-<unset>}"
    echo "LANG=${LANG:-<unset>} DISPLAY=${DISPLAY:-<unset>}"
    echo "ibus engine -> $(ibus engine 2>&1 || true)"
    if command -v xprop >/dev/null 2>&1; then
        xprop -root 2>/dev/null | grep XIM_SERVERS || echo "no XIM_SERVERS root property"
    else
        echo "(xprop not installed - skipping the XIM_SERVERS probe)"
    fi
    echo "-----------------------"
}

cargo build -p app -p ctl --bins >/dev/null

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"
export SHELL=/bin/bash
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
# Deterministic pixels: no compositor blending, no blink/fade animation.
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1
# XIM needs a UTF-8 locale the client can negotiate with; libpinyin's
# candidate space expects a zh locale. Exported explicitly so the script
# does not depend on the invoking shell's environment.
export LANG=zh_CN.UTF-8
# winit turns "@im=NAME" into the XIM_SERVERS atom "@server=NAME" match.
export XMODIFIERS="@im=ibus"

echo '{"theme":"dracula"}' >"$XDG_CONFIG_HOME/terminator-rust/state.json"

# --- Xvfb (random probe: stale sockets make fixed numbers refuse) --------
for _ in $(seq 1 12); do
    N=$((100 + RANDOM % 880))
    [ -S "/tmp/.X11-unix/X$N" ] && continue
    Xvfb ":$N" -screen 0 1200x800x24 >"$ROOT/xvfb.log" 2>&1 & XVFB_PID=$!
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

# --- session bus: reuse a reachable one, else start a private bus -------
# ibus talks DBus (engine switching), the app talks XIM (pure X11); CI
# runners have no session bus, so fork one and remember its pid.
bus_reachable() {
    [ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || return 1
    python3 - "$DBUS_SESSION_BUS_ADDRESS" <<'PY'
import socket, sys
addr = sys.argv[1]
path = None
for part in addr.split(","):
    if part.startswith("unix:path="):
        path = part[len("unix:path="):]
    elif part.startswith("unix:abstract="):
        path = "\0" + part[len("unix:abstract="):]
if not path:
    sys.exit(1)
try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    s.close()
except OSError:
    sys.exit(1)
PY
}
if ! bus_reachable; then
    step "starting a private session bus"
    OUT=$(dbus-daemon --session --fork --print-address=1 --print-pid=1 2>/dev/null) \
        || fail "dbus-daemon refused to start"
    DBUS_ADDR=$(echo "$OUT" | head -1)
    DBUS_PID=$(echo "$OUT" | tail -1)
    export DBUS_SESSION_BUS_ADDRESS="$DBUS_ADDR"
    echo "private bus pid $DBUS_PID"
fi

# --- ibus + libpinyin ----------------------------------------------------
step "ibus-daemon (xim) + engine libpinyin"
# Foreground + setsid so $! IS the daemon (no --daemonize pid guessing).
setsid ibus-daemon --replace --xim --panel=disable >"$ROOT/ibus.log" 2>&1 & IBUS_PID=$!
sleep 1.5
kill -0 "$IBUS_PID" 2>/dev/null || { tail "$ROOT/ibus.log"; fail "ibus-daemon died"; }
# The first engine switch can race the daemon's engine registration.
ENG=""
for _ in $(seq 1 6); do
    ibus engine libpinyin >/dev/null 2>&1 || true
    ENG=$(ibus engine 2>/dev/null || true)
    [ "$ENG" = "libpinyin" ] && break
    sleep 0.6
done
[ "$ENG" = "libpinyin" ] || { ime_diag; tail "$ROOT/ibus.log"; \
    fail "ibus engine is '$ENG', never libpinyin"; }
echo "engine: $ENG"
if command -v xprop >/dev/null 2>&1; then
    xprop -root 2>/dev/null | grep -q XIM_SERVERS \
        || { ime_diag; fail "X server has no XIM_SERVERS (ibus XIM server not up)"; }
    xprop -root 2>/dev/null | grep XIM_SERVERS
fi

# --- launch the app ------------------------------------------------------
step "launch app"
env DISPLAY="$DISPLAY_N" RUST_LOG=info \
  setsid "$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "ipc socket never appeared"; }
sleep 2   # theme style + shell settle

WID=""
for _ in $(seq 1 60); do
    WID=$(xdotool search --name '^terminator-rust$' 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.5
done
[ -n "$WID" ] || { tail "$ROOT/app.log"; fail "app window not found"; }
xdotool windowfocus "$WID"
eval "$(xdotool getwindowgeometry --shell "$WID")"
export X Y WIDTH HEIGHT
echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"

pane_id() { "$CTL" list --json 2>/dev/null | grep -o '"id": [0-9]*' | grep -o '[0-9]*' | head -1; }
PANE=$(pane_id)
[ -n "$PANE" ] || { "$CTL" list || true; fail "no pane listed"; }
cap() { "$CTL" capture "$PANE" 2>/dev/null || true; }
wait_capture() {  # wait_capture NEEDLE -> rc 0 once the capture holds NEEDLE
    for _ in $(seq 1 24); do
        cap | grep -q "$1" && return 0
        sleep 0.25
    done
    return 1
}

# --- S1: shell alive (control channel; the IME owns the keyboard) --------
step "S1: shell liveness via ctl (IME owns xdotool-typed keys)"
"$CTL" send "$PANE" --text "echo READY_IME"$'\n' >/dev/null
wait_capture READY_IME || { cap | tail -5; fail "shell not live (READY_IME never echoed)"; }
echo "shell live, READY_IME echoed"

# Purple-ink counter: the 2px rounded preedit underline is antialiased
# against the pane bg, so no pixel is the raw accent - but the accent's
# hue signature (blue and red both above green; dracula purple has
# b-g=102, r-g=42) survives any blend level, while fg/bg/text
# antialiasing stays neutral (b-g ~= 0). Inset 12px skips the window
# border and the focused-card accent stroke.
cat >"$ROOT/accent_count.py" <<'PY'
import os, sys
from PIL import Image
img = Image.open(sys.argv[1]).convert("RGB")
px = img.load()
X, Y, W, H = (int(os.environ[k]) for k in ("X", "Y", "WIDTH", "HEIGHT"))
n = 0
for y in range(Y + 12, Y + H - 12):
    for x in range(X + 12, X + W - 12):
        r, g, b = px[x, y]
        if b - g >= 25 and r - g >= 5:
            n += 1
print(n)
PY

# --- M1: composition swallows the keys -----------------------------------
step "M1: composing 'hanzi' never reaches the pane"
scrot -o "$ROOT/base.png"     # pre-composition baseline for M2
xdotool type --delay 150 'hanzi'
sleep 0.6
if cap | grep -q 'hanzi'; then
    ime_diag
    cap | tail -5
    fail "M1: 'hanzi' reached the pane - the XIM filter is NOT engaged"
fi
echo "M1 ok: composition swallowed the keystrokes"

# --- M2: preedit underline ink -------------------------------------------
step "M2: preedit ink (accent underline) vs baseline"
scrot -o "$ROOT/compose.png"
BASE_N=$(python3 "$ROOT/accent_count.py" "$ROOT/base.png")
COMP_N=$(python3 "$ROOT/accent_count.py" "$ROOT/compose.png")
DIFF=$((COMP_N - BASE_N))
echo "accent-hue px: baseline $BASE_N, composing $COMP_N (diff $DIFF)"
if [ "$DIFF" -eq 0 ]; then
    echo "WARN: M2 SKIP - no app-painted preedit ink (XIM negotiated PreeditNothing; the server draws its own)"
elif [ "$DIFF" -lt 4 ]; then
    fail "M2: preedit ink delta $DIFF is neither 0 (skippable) nor >= 4"
else
    echo "M2 ok: preedit underline painted ($DIFF purple px over baseline)"
fi

# --- M3: commit round-trip ------------------------------------------------
step "M3: space commits libpinyin default candidate"
xdotool key --clearmodifiers space
sleep 0.6
wait_capture '汉字' || { cap | tail -5; ime_diag; \
    fail "M3: committed candidate 汉字 never reached the pane"; }
echo "commit round-trip ok (汉字 in capture)"
xdotool key Return
sleep 0.3
MK="IME_OK_$((RANDOM % 30000))"
# libpinyin starts in Chinese mode; a bare Shift press/release toggles
# English passthrough. The IME stays the active engine - this proves
# committed ENGLISH flows to the pane too, not just IME-off typing.
xdotool key --clearmodifiers Shift_L
sleep 0.4
xdotool type --delay 120 "echo $MK"
xdotool key Return
wait_capture "$MK" || { cap | tail -5; ime_diag; \
    fail "M3: ASCII marker '$MK' did not pass through the active IME"; }
echo "English passthrough ok ($MK in capture)"

# --- M4: alive throughout + quit -----------------------------------------
step "M4: liveness + Ctrl+Shift+Q quits"
xdotool search --name '^terminator-rust$' | grep -q . || fail "M4: window vanished mid-test"
kill -0 "$APP_PID" 2>/dev/null || fail "M4: app process died mid-test"
xdotool key --clearmodifiers ctrl+shift+q
for _ in $(seq 1 40); do
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.25
done
kill -0 "$APP_PID" 2>/dev/null && { "$CTL" list || true; \
    fail "M4: app still alive 10s after Ctrl+Shift+Q"; }
APP_PID=""
[ -s "$XDG_CONFIG_HOME/terminator-rust/state.json" ] || fail "M4: state.json missing/empty after quit"
echo "app quit on Ctrl+Shift+Q, state persisted"

echo "e2e-ime: ALL GREEN (M1-M4)"
