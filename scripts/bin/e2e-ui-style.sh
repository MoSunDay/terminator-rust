#!/usr/bin/env bash
# Headless e2e for the themed UI surface of terminator-rust: real Xvfb
# rendering, verified pixel-exact with scrot + PIL against palette-derived
# expectations (dracula: fg 248,248,242 / bg 40,42,54 / highlight 189,147,249):
#   A: the global terminal background (settings bg "#ff0000" + glass 0.5)
#      paints every pane: #ff0000 at alpha 0.5 over the opaque theme-bg
#      page panel -> mix 50/50 with the theme bg (effective_bg /
#      pane_bg_alpha / with_opacity).
#   B: the split gutter is two-tone - chrome_bg field (mix(bg, fg, 0.045))
#      with a 2px rounded grab handle (mix(bg, fg, 0.13)) centered in the
#      strip (idle color; the hover step only paints under the pointer).
#   C: the background override is UNIFORM - the right pane paints the
#      same blended red as the left (appearance is global now, not
#      per-pane).
#   D: the chrome top bar paints chrome_bg (mix(bg, fg, 0.045)); sampled at
#      30% width, clear of the chips and the trailing icon buttons.
#   E: the active tab chip carries a 2px accent underline (rounded caps,
#      inset clear of the pill corners) flush at its bottom edge.
#   F: the active tab chip fill is the subtle accent tint mix(bg, accent,
#      0.18).
#   G: the pane header title is CENTERED in the space left of the X
#      close button (text bbox midpoint vs computed title_rect center);
#      the focused pane's header fill is the accent tint mix(bg, accent,
#      0.10).
#   H: the old centered window-title row is GONE - the chrome is a single
#      chip row (zoom/settings live at its right edge); no title-colored
#      text may render in the bare chrome zone (regression guard).
#   I: the chrome bar is ALWAYS on: with the last-but-one tab closed the
#      chip row stays (active-chip underline + fill still painted) and
#      the pane content is pushed below it (single-tab windows render
#      the same ~42px chrome as multi-tab ones).
# TERMINATOR_OPAQUE=1 pins window opacity 1.0: with no compositor in Xvfb
# the translucent fills would blend over garbage; pinned opaque the pixel
# expectations are byte-identical to the pre-transparency values.
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
DISPLAY_N=""                  # probed below (stale sockets break ":$$")
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
# No compositor in Xvfb: pin full opacity for deterministic pixels.
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1   # pin fades/cursor blink to end states
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# Preset: dracula theme, a vertical 50/50 Split PLUS a second single-pane
# tab, with the global terminal background set to #ff0000 at 50% glass -
# checks A/C assert the override applies UNIFORMLY to both panes (the
# appearance lives in settings now; the legacy per-pane "bg"/
# "transparency" keys below are dead weight serde ignores - kept to prove
# old files still load). Check I closes "extra" to verify the bar STAYS
# at one tab. Pane ids are re-allocated on load; manual_title "red" is
# the stable ctl addressing key.
cat > "$XDG_CONFIG_HOME/terminator-rust/state.json" <<'JSON'
{
  "theme": "dracula",
  "settings": { "split_axis": "v", "split_ratio": 0.5, "bg": "#ff0000", "transparency": 0.5 },
  "tabs": [
    { "title": "style", "focused": 1,
      "root": { "Split": { "axis": "v", "ratio": 0.5,
        "first":  { "Pane": { "id": 1, "meta": { "kind": "Local", "manual_title": "red",  "bg": null, "transparency": 0.0, "degraded": false } } },
        "second": { "Pane": { "id": 2, "meta": { "kind": "Local", "manual_title": null,   "bg": null,       "transparency": 0.0, "degraded": false } } } } } },
      { "title": "extra", "focused": 3,
        "root": { "Pane": { "id": 3, "meta": { "kind": "Local", "manual_title": null, "bg": null, "transparency": 0.0, "degraded": false } } } }
  ]
}
JSON

# --- Xvfb + app ----------------------------------------------------------
step "launch Xvfb + app"
# Random display probe: a fixed/PID-derived number can collide with a
# stale /tmp/.X11-unix socket left by an earlier crashed run.
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
[ "$("$CTL" list --json | grep -c '"id": ')" = "3" ] \
    || { "$CTL" list --json; fail "expected 3 panes (Split pair + 'extra')"; }
echo "preset loaded: Split pair + 'extra' tab, 'red' addressable"

# --- pixel assertions ----------------------------------------------------
step "scrot + PIL pixel assertions"
# Park the pointer on bare chrome first: Xvfb starts it at the screen
# center, exactly on the split gutter - the divider grab handle would
# paint its hover step and break the resting-gutter expectation.
xdotool mousemove $((X + WIDTH * 30 / 100)) $((Y + 6))
sleep 0.5   # let one 50ms repaint cadence land before capturing
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

# A: left pane deep area - the global bg override #ff0000 painted at
#    glass alpha pane_bg_alpha(0.5, 1.0)=0.5 over the opaque theme-bg
#    page panel: (40+0.5*215, 42-21, 54-27) = (147.5, 21, 27); rounding
#    may give 148. Kept as a literal: this exercises effective_bg +
#    pane_bg_alpha + with_opacity, not mix().
check("A blend-red-over-bg", int(X + W * 0.25), int(Y + H * 0.55), (147, 21, 27))

# C: the override is uniform - the right pane paints the same blended
#    red (appearance is global; a per-pane resurface would repaint the
#    plain theme bg here).
check("C uniform-bg", int(X + W * 0.75), int(Y + H * 0.55), (147, 21, 27))

# D: chrome top bar fill = mix(bg, fg, 0.045); x=30% is bare chrome between
#    the tab chips and the right-edge icon cells.
check("D chrome-bg", int(X + W * 0.30), Y + 4, mix(BG, FG, 0.045))

# B: gutter two-tone - chrome_bg field mix(bg, fg, 0.045) with a 2px
#    rounded grab handle mix(bg, fg, 0.13) centered in the strip. Scan the
#    band across the 50% split line: the 6px strip contributes a few chrome
#    field px and >=1 handle px (the rest of the band is pane content;
#    unfocused-pane card seams paint hairline mix(bg, fg, 0.09), distinct).
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
#    Geometry: single-row chrome = 4px inset + chip row Y+4..Y+32
#    (CHIP_H=28), so the underline sits at Y+30..Y+31; the wide band stops
#    short of the pane area (Y+42, hairline ~Y+40), where the focused
#    pane's accent stroke would false-positive.
hits = sum(1 for yy in range(Y + 21, Y + 33)
           for x in range(X, X + 220) if close(img.getpixel((x, yy)), ACCENT))
ok = hits >= 3
if not ok:
    rc = 1
print(f"E chip-underline: {hits} px ~= {ACCENT} in scan band [{'OK' if ok else 'FAIL'}]")

# F: active-chip fill = mix(bg, accent, 0.18), sampled inside the chip
#    (spans roughly x X..X+75, y Y+4..Y+32) off the label and close glyphs.
check("F chip-fill", X + 45, Y + 11, mix(BG, ACCENT, 0.18))

# G: pane header title centered. The focused "red" pane's title draws in
#    FG over the focused-header tint in the header strip right below the
#    chrome bar (pane area starts ~Y+42, header strip is 24px tall ->
#    scan the band). title_rect = [pane_left+8, pane_right-24(X btn)-8]
#    -> midpoint is the computed expectation; tolerance 6px for font
#    rounding.
title_text = mix(FG, BG, 0.38)
pane_w = (W - 6) / 2.0            # 50/50 split, 6px divider
scan_lo, scan_hi = X + 10, int(X + pane_w - 40)   # clear of the X button
hits = []
for x in range(scan_lo, scan_hi):
    for yy in range(Y + 44, Y + 62):
        px = img.getpixel((x, yy))
        if close(px, FG) or close(px, title_text):
            hits.append(x)
            break
ok = len(hits) >= 3
if ok:
    mid = (hits[0] + hits[-1]) / 2.0
    exp = X + 8 + (pane_w - 24 - 8) / 2.0
    ok = abs(mid - exp) <= 6
    print(f"G pane-title-center: text {hits[0]}..{hits[-1]} mid {mid:.0f} want {exp:.0f} [{'OK' if ok else 'FAIL'}]")
else:
    print(f"G pane-title-center: no title text found in scan [{'FAIL'}]")
if not ok:
    rc = 1

# H: no window-title row anymore. Scan the bare chrome zone right of the
#    trailing buttons (their glyphs span ~X+190..X+240) and left of the
#    zoom/inspector cells (start ~X+W-57), at the chip-row mid line:
#    expect ZERO title-colored pixels there.
hits = [x for x in range(X + 260, X + W - 60)
        if close(img.getpixel((x, Y + 18)), title_text)]
ok = len(hits) == 0
print(f"H no-topbar-title: {len(hits)} title-text px in chrome scan [{'OK' if ok else 'FAIL'}]")
if not ok:
    rc = 1

sys.exit(rc)
PY
step "pixel assertions passed"

# --- I: the chrome bar stays with a single tab ----------------------------
# Switch to the "extra" tab and close its only pane (Ctrl+Shift+W): the
# tab vanishes with it, leaving ONE tab - the bar must STAY (always-on
# chrome) and the pane content must start below it (~Y+42), not at y=0.
step "I: single tab keeps the chrome row"
xdotool key --clearmodifiers ctrl+Page_Down
sleep 0.5
xdotool key --clearmodifiers ctrl+shift+w
sleep 0.8
"$CTL" list --json | grep -q '"name": "red"' \
    || { "$CTL" list --json; fail "'red' pane lost while closing the extra tab"; }
[ "$("$CTL" list --json | grep -c '"id": ')" = "2" ] \
    || { "$CTL" list --json; fail "expected 2 panes after closing the extra tab"; }
scrot -o "$ROOT/scr2.png"
export SCR2="$ROOT/scr2.png"
python3 - <<'PY'
import os, sys
from PIL import Image

FG = (248, 248, 242)
BG = (40, 42, 54)
ACCENT = (189, 147, 249)

def mix(a, b, t):
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

img = Image.open(os.environ["SCR2"]).convert("RGB")
X, Y, W, H = (int(os.environ[k]) for k in ("X", "Y", "WIDTH", "HEIGHT"))
TOL = 3
rc = 0

def close(got, exp):
    return all(abs(g - e) <= TOL for g, e in zip(got, exp))

# I1: the chip row STAYS after the last-but-one tab closes: the former
#     chip band still holds the active-chip accent underline and the
#     active-chip fill. (Observed at one tab: fill Y+4..Y+29, underline
#     Y+30..Y+31 - inside the bands below. The focused pane's accent
#     frame only lives at the pane edges, never inside this band.)
accent_px = sum(1 for yy in range(Y + 21, Y + 33) for x in range(X + 10, X + 220)
                if close(img.getpixel((x, yy)), ACCENT))
chip_fill = mix(BG, ACCENT, 0.18)
fill_px = sum(1 for yy in range(Y + 6, Y + 30) for x in range(X + 10, X + 220)
              if close(img.getpixel((x, yy)), chip_fill))
ok = accent_px >= 1 and fill_px >= 1
if not ok:
    rc = 1
print(f"I bar-stays: accent {accent_px} px, chip-fill {fill_px} px in band "
      f"[{'OK' if ok else 'FAIL'}]")

# I2: content is pushed below the bar: the pixel where the focused pane
#     header used to start (y=0 layout) is now active-chip fill, NOT the
#     focused-header tint; the tint itself only appears lower down
#     (observed Y+44..Y+64; pane content starts ~Y+66).
exp_header = mix(BG, ACCENT, 0.10)
got = img.getpixel((X + 50, Y + 15))
ok = close(got, chip_fill)
if not ok:
    rc = 1
print(f"I content-pushed-down: (X+50,Y+15) got {got} want chip-fill {chip_fill} "
      f"[{'OK' if ok else 'FAIL'}]")
header_low = sum(1 for yy in range(Y + 42, Y + 71)
                 if close(img.getpixel((X + 50, yy)), exp_header))
if header_low < 1:
    rc = 1
print(f"I header-below-bar: {header_low} focused-header-tint px at x=X+50 in "
      f"Y+42..Y+70 [{'OK' if header_low >= 1 else 'FAIL'}]")

sys.exit(rc)
PY
step "single-tab chrome assertions passed"

echo "e2e-ui-style: ALL GREEN"
