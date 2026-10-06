#!/usr/bin/env bash
# Shared, isolated XIM + native IBus panel fixture. Source from the repo root.
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
BUILD_APP=1
if [ "$#" -gt 0 ]; then
    [ "$#" -eq 2 ] && [ "$1" = "--bundle" ] || {
        echo "usage: e2e-ime.sh [--bundle DIRECTORY]" >&2; exit 2;
    }
    BUNDLE=$(cd "$2" && pwd)
    APP="$BUNDLE/bin/terminator-rust"
    CTL="$BUNDLE/bin/terminator-ctl"
    BUILD_APP=0
fi
ROOT=$(mktemp -d /tmp/term-e2e-ime-XXXXXX)
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

if [ "$BUILD_APP" -eq 1 ]; then
    cargo build --target-dir target -p app -p ctl --bins >/dev/null
fi
[ -x "$APP" ] && [ -x "$CTL" ] || fail "test binaries are missing"

export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export XDG_CONFIG_HOME="$ROOT/config" XDG_CACHE_HOME="$ROOT/cache"
unset WAYLAND_DISPLAY
mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"
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

echo '{"theme":"dracula"}' >"$HOME/.terminator-rust/state.json"

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

# Always isolate the bus: --replace must never replace the user's daemon.
OUT=$(dbus-daemon --session --fork --print-address=1 --print-pid=1)
DBUS_PID=$(echo "$OUT" | tail -1)
export DBUS_SESSION_BUS_ADDRESS=$(echo "$OUT" | head -1)

# --- ibus + libpinyin ----------------------------------------------------
step "ibus-daemon (xim) + engine libpinyin"
# Foreground + setsid so $! IS the daemon (no --daemonize pid guessing).
setsid ibus-daemon --replace --xim --emoji-extension=disable >"$ROOT/ibus.log" 2>&1 & IBUS_PID=$!
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
