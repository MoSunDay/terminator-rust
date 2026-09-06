#!/usr/bin/env bash
# Headless e2e for the themed UI surface of terminator-rust: real Xvfb
# rendering, verified pixel-exact with scrot + PIL against palette-derived
# expectations (dracula: fg 248,248,242 / bg 40,42,54 / highlight 189,147,249):
#   A: per-pane background blended with transparency - left pane carries
#      bg #ff0000 + transparency 0.5 -> mix 50/50 with the theme bg
#      (blend_background / with_alpha_over).
#   B: the split divider strip renders the palette divider step
#      (render/colors.rs mix(bg, fg, 0.16) == (73,75,84)).
#   C: the plain right pane paints the theme background untouched.
#   D: the chrome top bar paints chrome_bg (mix(bg, fg, 0.07) == (55,56,67));
#      sampled at 30% width to dodge the centered title text.
# Plus a control-socket check that the preset Split state.json really loaded
# (two panes, the manual_title "red" survives as the ctl addressing key).
#
# Usage: scripts/bin/e2e-ui-style.sh   (from the repo root; needs Xvfb +
# xdotool + scrot + python3-PIL). Set E2E_KEEP=1 to keep the scratch dir.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-style-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
DISPLAY_N=":$$"          # unique per run; no clash with parallel scripts
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

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/bash
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# Preset: dracula theme, settings, and a vertical 50/50 Split whose left
# pane blends #ff0000 at 50% transparency. Pane ids are re-allocated on
# load; manual_title "red" is the stable ctl addressing key.
cat > "$XDG_CONFIG_HOME/terminator-rust/state.json" <<'JSON'
{
  "theme": "dracula",
  "settings": { "split_axis": "v", "split_ratio": 0.5 },
  "tabs": [
    { "title": "style", "focused": 1,
      "root": { "Split": { "axis": "v", "ratio": 0.5,
        "first":  { "Pane": { "id": 1, "meta": { "kind": "Local", "manual_title": "red",  "bg": "#ff0000", "transparency": 0.5, "degraded": false } } },
        "second": { "Pane": { "id": 2, "meta": { "kind": "Local", "manual_title": null,   "bg": null,       "transparency": 0.0, "degraded": false } } } } } }
  ]
}
JSON

# --- Xvfb + app ----------------------------------------------------------
step "launch Xvfb $DISPLAY_N + app"
Xvfb "$DISPLAY_N" -screen 0 1200x800x24 & XVFB_PID=$!
sleep 0.7
export DISPLAY="$DISPLAY_N"   # for xdotool + scrot (app gets it via env below)
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "ipc socket never appeared"; }
# capture/send is full remote control: the socket must be owner-only.
[ "$(stat -c %a "$SOCK")" = "600" ] || fail "socket perms $(stat -c %a "$SOCK") != 600"
sleep 2   # let the theme style + pane shells settle

# --- window geometry (absolute screen coords; do NOT assume 0,0) ---------
step "locate window"
WID=""
for _ in $(seq 1 20); do
    WID=$(xdotool search --name terminator-rust 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.25
done
[ -n "$WID" ] || { tail "$ROOT/app.log"; fail "app window not found"; }
xdotool windowfocus "$WID"   # no WM: give the window X focus
eval "$(xdotool getwindowgeometry --shell "$WID")"   # -> X Y WIDTH HEIGHT
export X Y WIDTH HEIGHT   # for the PIL checker below
echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"

# --- functional: the preset Split layout really loaded -------------------
step "ctl list shows both preset panes (manual_title 'red')"
"$CTL" list --json | grep -q '"name": "red"' \
    || { "$CTL" list --json; fail "pane 'red' missing - state.json layout not loaded"; }
[ "$("$CTL" list --json | grep -c '"id": ')" = "2" ] \
    || { "$CTL" list --json; fail "expected 2 panes from the preset Split"; }
echo "preset Split loaded: 2 panes, 'red' addressable"

# --- pixel assertions ----------------------------------------------------
step "scrot + PIL pixel assertions"
scrot -o "$ROOT/scr.png"
export SCRCAP="$ROOT/scr.png"
python3 - <<'PY'
import os, sys
from PIL import Image

img = Image.open(os.environ["SCRCAP"]).convert("RGB")
X, Y, W, H = (int(os.environ[k]) for k in ("X", "Y", "WIDTH", "HEIGHT"))
TOL = 3
rc = 0

def close(got, exp):
    return all(abs(g - e) <= TOL for g, e in zip(got, exp))

def check(name, x, y, exp):
    global rc
    got = img.getpixel((x, y))
    ok = close(got, exp)
    if not ok:
        rc = 1
    print(f"{name}: got {got} want {exp} [{'OK' if ok else 'FAIL'}]")

# A: left pane deep area - #ff0000 blended 50/50 over dracula bg
#    (40+0.5*215, 42-21, 54-27) = (147.5, 21, 27); rounding may give 148.
check("A blend-red-over-bg", int(X + W * 0.25), int(Y + H * 0.55), (147, 21, 27))

# C: right pane paints the plain theme background.
check("C theme-bg", int(X + W * 0.75), int(Y + H * 0.55), (40, 42, 54))

# D: chrome top bar fill = mix(bg, fg, 0.07); x=30% dodges the title text.
check("D chrome-bg", int(X + W * 0.30), Y + 4, (55, 56, 67))

# B: divider strip = mix(bg, fg, 0.16) == (73,75,84); count matching pixels
#    across the horizontal band around the 50% split line (>=4 of the 6px
#    strip; tolerance absorbs per-pixel rounding).
exp = (73, 75, 84)
y = int(Y + H * 0.55)
hits = sum(1 for x in range(int(X + W * 0.45), int(X + W * 0.55))
           if close(img.getpixel((x, y)), exp))
ok = hits >= 4
if not ok:
    rc = 1
print(f"B divider-strip: {hits} px ~= {exp} in scan band [{'OK' if ok else 'FAIL'}]")

sys.exit(rc)
PY
step "pixel assertions passed"

echo "e2e-ui-style: ALL GREEN"
