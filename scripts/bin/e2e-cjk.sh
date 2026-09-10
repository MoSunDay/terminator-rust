#!/usr/bin/env bash
# Headless e2e for embedded CJK font support (Linux UI parity; the font is
# compiled in, so macOS/Windows render identically by construction):
#   1. Asset: the embedded subset's cmap really covers the CJK ranges
#      (checked with fontTools over assets/fonts/*.otf - deterministic).
#   2. Live: boot the app (a corrupt/unparseable font would panic inside
#      epaint at first layout), send `echo 汉字测试` over the control
#      socket into a real pty, and assert `ctl capture` round-trips the
#      exact UTF-8 line (wide cells + spacer tails intact).
#   3. Pixels: scrot + PIL. Real Han glyphs painted at the measured wide
#      scale have ink ~14x15px with interior strokes; ASCII at 14pt stays
#      <= 9x10.5px, and a tofu fallback (hollow replacement square) has an
#      empty interior. So: >= 6 connected ink components with w>=10.5,
#      h>=12.5, and >= 3 of them carrying >= 8 interior ink pixels
#      (the command-echo line alone yields 4 such glyphs).
# Usage: scripts/bin/e2e-cjk.sh  (repo root; needs Xvfb + xdotool + scrot +
# python3-PIL + python3-fontTools). E2E_KEEP=1 keeps the scratch dir.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-cjk-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
DISPLAY_N=":$$"
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

# --- 1. asset cmap coverage (fontTools, no X needed) ----------------------
step "embedded subset cmap covers CJK sample set"
python3 - <<'PY' || fail "fontTools cmap check"
import sys
try:
    from fontTools.ttLib import TTFont
except ImportError:
    sys.exit("python3-fontTools missing")
f = TTFont("assets/fonts/NotoSansSC-Regular-subset.otf")
cmap = f.getBestCmap()
sample = ("汉字测试简繁體かなカナ・。「」【】《》～！？（）"
          "Ａｚ０９Ⅰ①㈱ㄅㄆㅏㅣ纮㙟")
missing = [c for c in sample if ord(c) not in cmap]
if missing:
    sys.exit(f"cmap missing: {missing!r}")
print(f"asset cmap OK ({f['maxp'].numGlyphs} glyphs)")
PY

# --- 2. live boot ----------------------------------------------------------
export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/bash
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# Single dracula pane with a stable ctl addressing key.
cat > "$XDG_CONFIG_HOME/terminator-rust/state.json" <<'JSON'
{
  "theme": "dracula",
  "settings": { "split_axis": "v", "split_ratio": 0.5 },
  "tabs": [
    { "title": "cjk", "focused": 0,
      "root": { "Pane": { "id": 1, "meta": { "kind": "Local", "manual_title": "cjk", "bg": null, "transparency": 0.0, "degraded": false } } } }
  ]
}
JSON

# Random display probe: a fixed/PID-derived number can collide with a
# stale /tmp/.X11-unix socket left by an earlier crashed run.
for _ in $(seq 1 12); do
    N=$((100 + RANDOM % 880))
    DISPLAY_N=":$N"
    [ -S "/tmp/.X11-unix/X$N" ] && continue
    Xvfb "$DISPLAY_N" -screen 0 1200x800x24 & XVFB_PID=$!
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
sleep 2   # theme style + shell settle

step "locate window"
WID=""
for _ in $(seq 1 60); do
    WID=$(xdotool search --name terminator-rust 2>/dev/null | head -1 || true)
    [ -z "$WID" ] && WID=$(xdotool search --class terminator-rust 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.5
done
[ -n "$WID" ] || { tail "$ROOT/app.log"; fail "app window not found"; }
xdotool windowfocus "$WID"
eval "$(xdotool getwindowgeometry --shell "$WID")"
export X Y WIDTH HEIGHT
echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"

# --- 3. control-channel CJK round-trip ------------------------------------
step "send CJK echo, capture round-trips"
"$CTL" send cjk --text "echo 汉字测试"$'\n' >/dev/null
CAP=""
for _ in $(seq 1 20); do
    CAP=$("$CTL" capture cjk 2>/dev/null || true)
    echo "$CAP" | grep -q "汉字测试" && break
    sleep 0.25
done
echo "$CAP" | grep -q "汉字测试" || { echo "$CAP"; fail "capture lacks 汉字测试"; }
echo "capture round-trip OK"

# --- 4. pixel assertions ----------------------------------------------------
step "scrot + PIL ink-component assertions"
scrot -o "$ROOT/scr.png"
export SCRCAP="$ROOT/scr.png"
python3 - <<'PY'
import os, sys
from PIL import Image

FG = (248, 248, 242)   # dracula foreground

def is_fg(p):
    return abs(p[0]-FG[0])+abs(p[1]-FG[1])+abs(p[2]-FG[2]) < 180

img = Image.open(os.environ["SCRCAP"])
X, Y, W, H = (int(os.environ[k]) for k in ("X", "Y", "WIDTH", "HEIGHT"))
px = img.load()

# Content region: below the chrome chip row and the pane header.
x0, x1 = X + 8, X + W - 8
y0, y1 = Y + 55, Y + H - 8

# Ink components = (column run) x (row run) inside each band. Column runs
# come from any-ink-in-band; the row run then isolates the text line, so a
# glyph never inherits rows from a stacked line sharing its columns (CJK
# ink at the wide scale overflows the ASCII line height and lines touch).
rows = [y for y in range(y0, y1) if any(is_fg(px[x, y]) for x in range(x0, x1, 2))]
bands = []
for y in rows:
    if bands and y - bands[-1][1] <= 3:
        bands[-1][1] = y
    else:
        bands.append([y, y])

comps = []   # (x_min, x_max, y_min, y_max)
for (by0, by1) in bands:
    cols = []
    for x in range(x0, x1):
        hit = any(is_fg(px[x, y]) for y in range(by0, by1 + 1))
        if cols and hit and x - cols[-1][1] <= 1:
            cols[-1][1] = x
        elif hit:
            cols.append([x, x])
    for (cx0, cx1) in cols:
        inked = [y for y in range(by0, by1 + 1)
                 if any(is_fg(px[x, y]) for x in range(cx0, cx1 + 1))]
        yr = []
        for y in inked:
            if yr and y - yr[-1][1] <= 2:
                yr[-1][1] = y
            else:
                yr.append([y, y])
        for (ry0, ry1) in yr:
            comps.append((cx0, cx1, ry0, ry1))

wide = [c for c in comps
        if 9.5 <= c[1]-c[0] <= 22 and 12.5 <= c[3]-c[2] <= 22]
def interior_ink(c):
    cx0, cx1, cy0, cy1 = c
    mx, my = (cx1-cx0)*0.22, (cy1-cy0)*0.22
    ix = range(int(cx0+mx), int(cx1-mx)+1)
    iy = range(int(cy0+my), int(cy1-my)+1)
    return sum(1 for x in ix for y in iy if is_fg(px[x, y]))
inked = [c for c in wide if interior_ink(c) >= 8]

ok_w = len(wide) >= 4
ok_i = len(inked) >= 3
print(f"ink components: {len(comps)} total, wide {len(wide)} "
      f"[{'OK' if ok_w else 'FAIL'}], stroked {len(inked)} "
      f"[{'OK' if ok_i else 'FAIL'}]")
if not ok_w:
    print("wide comps:", wide[:10])
sys.exit(0 if ok_w and ok_i else 1)
PY
step "pixel assertions passed"

echo "e2e-cjk: ALL GREEN"
