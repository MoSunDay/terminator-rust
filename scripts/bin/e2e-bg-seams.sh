#!/usr/bin/env bash
# Headless e2e for cell-background painting: ANSI background runs must
# render SEAM-FREE. epaint feathers every rect_filled edge, and the old
# draw_frame painted one rect PER CELL, so same-color regions showed a
# lattice of faint vertical/horizontal seams (1/255 off per channel at
# the cell pitch) plus a wrong-colored pane-bg strip on the right edge
# (the grid is floor(pane_w/cell_w) columns wide, the remainder showed
# the pane bg). draw_frame now paints MERGED runs (render/bg_runs.rs).
# Checks (scrot + PIL, dracula pane bg 40,42,54):
#   K1 vertical seams: every column of a full-width colored row must
#      match the band color within 1 per channel. Exactly-1-off noise
#      hits ~20 screen-fixed columns on ANY fill (GPU dither, measured
#      on the plain pane-bg rect too) - only >=2-off counts. Before the
#      fix: >=2-off columns at the cell pitch (9px).
#   K2 horizontal seams: bands are printed 2 rows tall; every row of a
#      column scan through the band interior must match the band color
#      (same tolerance; the old per-row rects left seam rows).
#   K3 right-edge bleed: full-width runs cover the pane's sub-cell right
#      remainder - the red band must reach pane_right EXACTLY (before
#      the fix it stopped at cols*cell.w, ~3px short at the 1200px
#      window; 1200 is not a multiple of the 9px cell).
#   K4 CJK wide-cell bg: inside the magenta CJK run (汉字测试 x2) there
#      are ZERO pane-bg pixels (wide cells used to leave holes in their
#      background).
# Expected band colors are the libghostty default ANSI palette (the VT
# engine owns cell colors); tolerance 8 covers GPU dither noise.
# TERMINATOR_OPAQUE=1 pins window opacity 1.0 (no compositor in Xvfb).
#
# Usage: scripts/bin/e2e-bg-seams.sh   (repo root; needs Xvfb + xdotool
# + scrot + python3-PIL). Set E2E_KEEP=1 to keep the scratch dir,
# E2E_BG_DEBUG=1 to dump every detected band.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-bg-XXXXXX)
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

export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/bash
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
export TERMINATOR_OPAQUE=1      # no compositor: pin opaque pixels
export TERMINATOR_NO_MOTION=1   # pin cursor blink to end states
export TERMINATOR_RESTORE=1     # load the preset below
export WINIT_X11_SCALE_FACTOR=1 # deterministic cell metrics
mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# Preset: dracula, one tab, ONE pane (id 1) - no bg override, the bands
# below exercise ANSI CELL backgrounds.
cat > "$HOME/.terminator-rust/state.json" <<'JSON'
{
  "theme": "dracula",
  "settings": { "split_axis": "v" },
  "tabs": [
    { "title": "bg", "focused": 1,
      "root": { "Pane": { "id": 1, "meta": { "kind": "Local", "degraded": false } } } }
  ]
}
JSON

# --- Xvfb + app ----------------------------------------------------------
step "launch Xvfb + app"
for _ in $(seq 1 12); do
    N=$((100 + RANDOM % 880))
    DISPLAY_N=":$N"
    [ -S "/tmp/.X11-unix/X$N" ] && continue
    Xvfb "$DISPLAY_N" -screen 0 1200x800x24 >/dev/null 2>&1 & XVFB_PID=$!
    sleep 0.7
    if kill -0 "$XVFB_PID" 2>/dev/null && DISPLAY="$DISPLAY_N" xdpyinfo >/dev/null 2>&1; then
        break
    fi
    kill "$XVFB_PID" 2>/dev/null || true
    XVFB_PID=""
done
[ -n "$XVFB_PID" ] || fail "no free X display"
export DISPLAY="$DISPLAY_N"   # for xdotool + scrot
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "ipc socket never appeared"; }
sleep 2   # theme style + pane shell settle

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
export X Y WIDTH HEIGHT
echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"

# --- paint the bands -------------------------------------------------------
step "print full-width colored rows (2 rows x3), blank, CJK magenta"
# --text does NOT append Enter: every payload ends with a real \n.
# Rows are sized from stty so each band is EXACTLY full-width lines
# (a fixed space count would leave a short wrapped row behind).
"$CTL" send 1 --text $'C=$(stty size | cut -d \' \' -f2)\n' >/dev/null
"$CTL" send 1 --text $'printf \'\\033[41m\'; head -c $((C*2)) /dev/zero | tr \'\\0\' \' \'; printf \'\\033[0m\\n\'\n' >/dev/null
"$CTL" send 1 --text $'printf \'\\033[44m\'; head -c $((C*2)) /dev/zero | tr \'\\0\' \' \'; printf \'\\033[0m\\n\'\n' >/dev/null
"$CTL" send 1 --text $'printf \'\\033[48;5;208m\'; head -c $((C*2)) /dev/zero | tr \'\\0\' \' \'; printf \'\\033[0m\\n\'\n' >/dev/null
"$CTL" send 1 --text $'printf \'\\n\'\n' >/dev/null
"$CTL" send 1 --text $'printf \'\\033[45m汉字测试汉字测试\\033[0m\\n\'\n' >/dev/null
# verify the payload really landed (CJK row + no leftover band text)
CAP=""
for _ in $(seq 1 20); do
    # default --lines 80 >= rows: frame_text returns the BOTTOM N rows,
    # and the fresh shell's content starts at the TOP of the screen.
    CAP=$("$CTL" capture 1 2>/dev/null || true)
    echo "$CAP" | grep -q "汉字测试汉字测试" && break
    sleep 0.25
done
echo "$CAP" | grep -q "汉字测试汉字测试" \
    || { echo "$CAP"; fail "capture lacks the CJK magenta row"; }
echo "capture round-trip OK (CJK row visible)"

step "scrot + PIL seam assertions"
# Park the pointer on bare chrome first: Xvfb starts it at the screen
# center (inside the pane - hover ink/cursor state must not move).
xdotool mousemove $((X + WIDTH * 30 / 100)) $((Y + 6))
sleep 0.5   # let one 50ms repaint cadence land before capturing
scrot -o "$ROOT/scr.png"
export SCRCAP="$ROOT/scr.png"
python3 - <<'PY'
import os, sys
from collections import Counter
from PIL import Image

# Dracula pane bg (theme), and the libghostty default ANSI palette the
# VT engine resolves SGR 41/44/48;5;208/45 into (exact-revision pinned).
PANE_BG = (40, 42, 54)
BANDS = [
    ("red", (204, 102, 102)),    # SGR 41
    ("blue", (129, 162, 190)),   # SGR 44
    ("orange", (255, 135, 0)),   # SGR 48;5;208
]
MAGENTA = (178, 148, 187)     # SGR 45 (pinned from a probe run)

def close(p, e, tol):
    return all(abs(a - b) <= tol for a, b in zip(p, e))

img = Image.open(os.environ["SCRCAP"]).convert("RGB")
px = img.load()
X, Y, W, H = (int(os.environ[k]) for k in ("X", "Y", "WIDTH", "HEIGHT"))
rc = 0
DBG = os.environ.get("E2E_BG_DEBUG") == "1"

# --- find the full-width bands: rows whose modal interior color is one
# of the expected ones with >90% dominance; group contiguous rows.
found = []   # (name, color, y0, y1)
y = Y + 44   # below the chrome (~42) and the pane header band
runs = []
while y < Y + H - 2:
    row = [px[x, y] for x in range(X + 4, X + W - 4)]
    modal, n = Counter(row).most_common(1)[0]
    if n > len(row) * 0.9 and not close(modal, PANE_BG, 8):
        hit = next((b for b in BANDS if close(modal, b[1], 8)), None)
        if DBG:
            print(f"debug: y={y} modal={modal} hit={hit}")
        if hit and runs and runs[-1][0] == hit[0] and y - runs[-1][3] <= 1:
            runs[-1][3] = y
        elif hit:
            runs.append([hit[0], hit[1], y, y])
    y += 1
for name, color, y0, y1 in runs:
    found.append((name, color, y0, y1))
if DBG:
    print("debug: bands:", found)

names = [f[0] for f in found]
if names != ["red", "blue", "orange"]:
    print(f"band detection: found {found}, want red/blue/orange in order")
    rc = 1

def band_color(b):
    # modal color re-measured on the band's middle row (GPU dithering
    # can make the modal pixel value off by 1 from the expectation).
    ym = (b[2] + b[3]) // 2
    row = [px[x, ym] for x in range(X + 4, X + W - 4)]
    return Counter(row).most_common(1)[0][0]

# --- pane edges: the blank row between the orange band and the CJK row
# is pure pane bg - walk outward from its center for the pane bounds.
orange = next((b for b in found if b[0] == "orange"), None)
if orange is None:
    print("K*: no orange band, cannot locate the blank row")
    sys.exit(1)
blank_y = orange[3] + 1
best_y, best_frac = blank_y, 0.0
for yy in range(orange[3] + 1, min(orange[3] + 20, Y + H - 2)):
    frac = sum(close(px[x, yy], PANE_BG, 2) for x in range(X, X + W, 4)) / (W / 4)
    if frac > best_frac:
        best_y, best_frac = yy, frac
ymid = X + W // 2
pane_left = ymid
while pane_left > X and close(px[pane_left - 1, best_y], PANE_BG, 2):
    pane_left -= 1
pane_right = ymid
while pane_right < X + W - 1 and close(px[pane_right + 1, best_y], PANE_BG, 2):
    pane_right += 1
print(f"pane x {pane_left}..{pane_right} (blank row y={best_y}, frac {best_frac:.2f})")

# --- K1 vertical seams: every interior column of each band's middle
#     row matches the band color within 1 (GPU dither noise is 1-off at
#     screen-fixed columns even on a single rect - measured on the plain
#     pane-bg fill; the per-cell lattice shows as >=2-off columns).
for b in found:
    ym = (b[2] + b[3]) // 2
    modal = band_color(b)
    strong = [x for x in range(pane_left, pane_right + 1)
              if max(abs(a - c) for a, c in zip(px[x, ym], modal)) >= 2]
    ok = not strong
    if not ok:
        rc = 1
    print(f"K1 {b[0]}: {len(strong)}/{pane_right - pane_left + 1} columns off "
          f">=2 (modal {modal}) [{'OK' if ok else 'FAIL'}]")

# --- K2 horizontal seams: a column scan through each band interior -
#     every row must match within 1 (the 2-row band is ONE merged rect;
#     the old per-row rects left 1-off seam rows at row boundaries).
xscan = (pane_left + pane_right) // 2
for b in found:
    modal = band_color(b)
    strong = [yy for yy in range(b[2] + 2, b[3] - 1)
              if max(abs(a - c) for a, c in zip(px[xscan, yy], modal)) >= 2]
    ok = not strong
    if not ok:
        rc = 1
    print(f"K2 {b[0]}: {len(strong)} rows off >=2 at x={xscan} [{'OK' if ok else 'FAIL'}]")

# --- K3 right-edge bleed: the red band must cover the pane's sub-cell
#     right remainder (before the fix it stopped at cols*cell.w).
if (red := next((b for b in found if b[0] == "red"), None)) is not None:
    ym = (red[2] + red[3]) // 2
    modal = band_color(red)
    right = pane_right
    while right > pane_left and not close(px[right, ym], modal, 8):
        right -= 1
    ok = right >= pane_right   # full-width runs bleed INTO the remainder
    if not ok:
        rc = 1
    print(f"K3 bleed: red covers to x={right}, pane_right={pane_right} "
          f"(gap {pane_right - right}px, want 0) [{'OK' if ok else 'FAIL'}]")

# --- K4 CJK wide-cell bg: zero pane-bg pixels inside the magenta run
#     (wide-cell backgrounds must not leave holes).
mag = [(x, yy) for yy in range(orange[3] + 1, Y + H - 2)
       for x in range(pane_left, pane_right + 1)
       if close(px[x, yy], MAGENTA, 12)]
if not mag:
    print(f"K4: no magenta CJK run found (expect ~{MAGENTA})")
    rc = 1
else:
    x0 = min(p[0] for p in mag)
    x1 = max(p[0] for p in mag)
    y0 = min(p[1] for p in mag)
    y1 = max(p[1] for p in mag)
    # Shrink past the run rect's feathered rim (epaint antialiases the
    # 1px edge into neighbors); a HOLE is a pane-bg px strictly inside.
    holes = [(x, yy) for yy in range(y0 + 3, y1 - 2) for x in range(x0 + 3, x1 - 2)
             if close(px[x, yy], PANE_BG, 2)]
    ok = not holes
    if not ok:
        rc = 1
    print(f"K4 cjk-bg: magenta run x{x0}..{x1} y{y0}..{y1}, "
          f"{len(holes)} pane-bg holes [{'OK' if ok else 'FAIL'}]")

sys.exit(rc)
PY
step "seam assertions passed"

echo "e2e-bg-seams: ALL GREEN"
