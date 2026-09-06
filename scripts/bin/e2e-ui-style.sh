#!/usr/bin/env bash
# Headless e2e for the themed UI surface of terminator-rust: real Xvfb
# rendering, verified pixel-exact with scrot + PIL against palette-derived
# expectations (dracula: fg 248,248,242 / bg 40,42,54 / highlight 189,147,249):
#   A: per-pane background blended with transparency - left pane carries
#      bg #ff0000 + transparency 0.5 -> mix 50/50 with the theme bg
#      (blend_background / with_alpha_over).
#   B: the split gutter is two-tone - chrome_bg field (mix(bg, fg, 0.045))
#      with one 1px divider line (mix(bg, fg, 0.13)) through the middle.
#   C: the plain right pane paints the theme background untouched.
#   D: the chrome top bar paints chrome_bg (mix(bg, fg, 0.045)); sampled at
#      30% width to dodge the centered title text.
#   E: the active tab chip carries a 2px accent underline flush at its
#      bottom edge.
#   F: the active tab chip fill is the subtle accent tint mix(bg, accent,
#      0.18).
#   G: the pane header title is CENTERED in the space left of the C/T/X
#      buttons (text bbox midpoint vs computed title_rect center).
#   H: the top-bar window title is optically centered: centered over the
#      area LEFT of the 41px inspector/zoom icon zone, not the full width.
# Every expected chrome color below is COMPUTED from the dracula constants
# with a mix() helper, so token retunes only touch render/colors.rs (this
# script changes only when geometry changes).
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

# Dracula constants; all chrome expectations are COMPUTED from these so a
# token retune in render/colors.rs never requires editing this block.
FG = (248, 248, 242)
BG = (40, 42, 54)
ACCENT = (189, 147, 249)

def mix(a, b, t):
    """Per-channel lerp a -> b by t, rounded (mirrors colors::mix)."""
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

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
#    Kept as a literal: this exercises blend_background, not mix().
check("A blend-red-over-bg", int(X + W * 0.25), int(Y + H * 0.55), (147, 21, 27))

# C: right pane paints the plain theme background.
check("C theme-bg", int(X + W * 0.75), int(Y + H * 0.55), BG)

# D: chrome top bar fill = mix(bg, fg, 0.045); x=30% dodges the (now
#    optically centered, see H) title text.
check("D chrome-bg", int(X + W * 0.30), Y + 4, mix(BG, FG, 0.045))

# B: gutter two-tone - chrome_bg field mix(bg, fg, 0.045) with a 1px
#    divider line mix(bg, fg, 0.13) down the strip middle. Scan the band
#    across the 50% split line: the 6px strip contributes a few chrome
#    field px and >=1 divider-line px (the rest of the band is pane content).
exp_div = mix(BG, FG, 0.13)
exp_fld = mix(BG, FG, 0.045)
y = int(Y + H * 0.55)
band = [img.getpixel((x, y)) for x in range(int(X + W * 0.45), int(X + W * 0.55))]
div_hits = sum(1 for px in band if close(px, exp_div))
fld_hits = sum(1 for px in band if close(px, exp_fld))
ok = div_hits >= 1 and fld_hits >= 2
if not ok:
    rc = 1
print(f"B gutter two-tone: {div_hits} px ~= {exp_div}, "
      f"{fld_hits} px ~= {exp_fld} in scan band [{'OK' if ok else 'FAIL'}]")

# E: active-chip underline = accent, 2px tall at the chip bottom edge.
#    Geometry: title row 22 + chip gap 3 + item_spacing 6 -> chip row
#    Y+31..Y+55 (CHIP_H=24), so the underline sits at Y+53..Y+55; the wide
#    band stops short of the pane area (Y+58), where the focused pane's
#    accent stroke would false-positive.
hits = sum(1 for yy in range(Y + 45, Y + 58)
           for x in range(X, X + 220) if close(img.getpixel((x, yy)), ACCENT))
ok = hits >= 3
if not ok:
    rc = 1
print(f"E chip-underline: {hits} px ~= {ACCENT} in scan band [{'OK' if ok else 'FAIL'}]")

# F: active-chip fill = mix(bg, accent, 0.18), sampled inside the chip
#    (spans roughly x X..X+75, y Y+31..Y+55) off the label and close glyphs.
check("F chip-fill", X + 45, Y + 37, mix(BG, ACCENT, 0.18))

# G: pane header title centered. The focused "red" pane's title draws in
#    FG over chrome_bg in the header strip right below the chrome bar
#    (pane area starts ~Y+58, header strip is 24px tall -> scan the band).
#    title_rect = [pane_left+8, pane_right-56(buttons)-8] -> midpoint is
#    the computed expectation; tolerance 6px for font rounding.
title_text = mix(FG, BG, 0.42)
pane_w = (W - 6) / 2.0            # 50/50 split, 6px divider
scan_lo, scan_hi = X + 10, int(X + pane_w - 80)   # clear of buttons
hits = []
for x in range(scan_lo, scan_hi):
    for yy in range(Y + 62, Y + 78):
        px = img.getpixel((x, yy))
        if close(px, FG) or close(px, title_text):
            hits.append(x)
            break
ok = len(hits) >= 3
if ok:
    mid = (hits[0] + hits[-1]) / 2.0
    exp = X + 8 + (pane_w - 64 - 8) / 2.0
    ok = abs(mid - exp) <= 6
    print(f"G pane-title-center: text {hits[0]}..{hits[-1]} mid {mid:.0f} want {exp:.0f} [{'OK' if ok else 'FAIL'}]")
else:
    print(f"G pane-title-center: no title text found in scan [{'FAIL'}]")
if not ok:
    rc = 1

# H: top-bar title optically centered over [left .. right-41] (the 41px
#    right zone holds the zoom + inspector icon cells).
hits = [x for x in range(X + 4, X + W - 44)
        if close(img.getpixel((x, Y + 11)), title_text)]
ok = len(hits) >= 3
if ok:
    mid = (hits[0] + hits[-1]) / 2.0
    exp = X + (W - 41) / 2.0
    ok = abs(mid - exp) <= 6
    print(f"H topbar-title-center: text {hits[0]}..{hits[-1]} mid {mid:.0f} want {exp:.0f} [{'OK' if ok else 'FAIL'}]")
else:
    print(f"H topbar-title-center: no title text found in scan [{'FAIL'}]")
if not ok:
    rc = 1

sys.exit(rc)
PY
step "pixel assertions passed"

echo "e2e-ui-style: ALL GREEN"
