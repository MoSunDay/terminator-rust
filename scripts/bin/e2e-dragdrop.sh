#!/usr/bin/env bash
# Headless e2e for the two drag-and-drop gestures of terminator-rust,
# driven through REAL X events (Xvfb + xdotool) with pixel asserts via
# scrot + PIL and tree asserts via the live state.json:
#   D1: Ctrl+drag a pane HEADER onto the right half of the sibling pane
#       (edge zone) - mid-drag the dropzone overlay must paint the
#       landing preview fill mix(bg, accent, 0.22) over the target's
#       right share ONLY; on release the pane detaches and re-splits
#       (root: v(pane2 | pane1) at the settings ratio).
#   D2: Ctrl+drag that pane's header onto the sibling's exact CENTER
#       (center zone) - release swaps the two leaf ids in place,
#       shape/ratio unchanged.
#   D3: drag the ACTIVE tab chip past the second chip - the tab order
#       reorders live and persists (titles ["beta","alpha"]), and the
#       dragged tab stays the active one.
#   D5: PLAIN primary drag (no modifiers) of a pane header onto the
#       sibling's right half - same move as D1: with >=2 panes in the
#       tab the bare gesture rearranges panes (a lone pane would keep
#       the OS-window drag).
#   D6: mid pane-drag, DWELL on another tab's chip (>=0.4s) to switch
#       tabs, then drop on the target pane's right EDGE: the pane
#       migrates across tabs (mid-drag preview fill asserted with the
#       source pane no longer on screen).
#   D7: dwell-switch to the other tab and drop on the target pane's
#       exact CENTER: the two panes swap leaf slots ACROSS tabs (pure
#       id swap, no re-split).
#   D8: drag the LAST pane of a tab away (ctrl held through the press:
#       a lone pane's bare header drag would move the OS window): the
#       emptied source tab closes itself.
#   D4: the app is still alive after all gestures; Ctrl+Shift+Q quits it.
# Preset (persist.rs serde tags, ids remapped on load in preorder): one
# window, tab "alpha" = v-split 0.5 with panes 1,2, tab "beta" = pane 3.
# All expected colors are COMPUTED from the dracula constants with a
# mix() helper mirroring render/colors.rs::mix.
# Usage: scripts/bin/e2e-dragdrop.sh   (repo root; needs Xvfb + xdotool +
# scrot + python3-PIL). Set E2E_KEEP=1 to keep the scratch dir.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-dd-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
STATE_REL=".terminator-rust/state.json"
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

fail() { echo "FAIL: $*" >&2; tail -20 "$ROOT/app.log" 2>/dev/null || true; exit 1; }
step() { echo "== $*"; }

# Build first: the sandboxed HOME below would hide rustup/toolchains.
cargo build -p app -p ctl --bins >/dev/null

export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/bash
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
STATE="$HOME/$STATE_REL"
# No compositor in Xvfb: pin full opacity for deterministic pixels, and
# pin hover fades / cursor blink to their end states.
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1
# e2e presets rely on session restore; the default launch is a fresh tab
export TERMINATOR_RESTORE=1
mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# Preset: dracula, one window; tab "alpha" = vertical 50/50 split (panes
# remapped to 1,2 in preorder on load), tab "beta" = single pane (3).
cat > "$STATE" <<'JSON'
{
  "theme": "dracula",
  "settings": { "split_axis": "v", "split_ratio": 0.5 },
  "windows": [
    { "id": 1, "active_tab": 0, "tabs": [
      { "title": "alpha", "focused": 11,
        "root": { "Split": { "axis": "v", "ratio": 0.5,
          "first":  { "Pane": { "id": 11, "meta": { "kind": "Local", "manual_title": null, "bg": null, "transparency": 0.0, "degraded": false } } },
          "second": { "Pane": { "id": 12, "meta": { "kind": "Local", "manual_title": null, "bg": null, "transparency": 0.0, "degraded": false } } } } } },
      { "title": "beta", "focused": 13,
        "root": { "Pane": { "id": 13, "meta": { "kind": "Local", "manual_title": null, "bg": null, "transparency": 0.0, "degraded": false } } } }
    ] }
  ]
}
JSON

# --- Xvfb + app ----------------------------------------------------------
step "launch Xvfb + app"
# Random display probe: a fixed/PID-derived number can collide with a
# stale /tmp/.X11-unix socket left by an earlier crashed run.
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
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid nohup "$APP" </dev/null >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || fail "ipc socket never appeared"
sleep 2   # let the theme style + pane shells settle

# --- window geometry (absolute screen coords; do NOT assume 0,0) ---------
step "locate window"
WID=""
for _ in $(seq 1 20); do
    WID=$(xdotool search --onlyvisible --name terminator-rust 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.25
done
[ -n "$WID" ] || fail "app window not found"
xdotool windowfocus "$WID"   # no WM: give the window X focus
eval "$(xdotool getwindowgeometry --shell "$WID")"   # -> X Y WIDTH HEIGHT
export X Y WIDTH HEIGHT   # for the PIL checkers below
echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"
kill -0 "$APP_PID" 2>/dev/null || fail "app died during startup"

# --- derived geometry (mirrors layout_tab / split_rect_gapped) -----------
# Single-row chrome = 4 + chip 28 + 4 = 36px nominal (egui adds ~5px of
# panel/item spacing; only the derived header/gut origins matter below,
# both land inside their targets with that slack). A
# v-split at 0.5 with a 6px divider: left w = (W-6)/2, right the rest.
CHROME_H=36; GUT=6
AX=$X; AY=$((Y + CHROME_H)); AW=$WIDTH; AH=$((HEIGHT - CHROME_H))
W1=$(((AW - GUT) / 2))
P2X=$((AX + W1 + GUT)); P2W=$((AW - W1 - GUT))
HX1=$((AX + W1 / 2))            # pane 1 header center (left half)
HY1=$((AY + 12))
MIDY=$((AY + AH / 2))           # vertical midline of the pane area
export MIDY
echo "pane1 ${W1}px | gutter | pane2 ${P2W}px; area ${AW}x${AH}"

step "sanity: preset loaded (3 panes, alpha split)"
[ "$("$CTL" list --json | grep -c '"id": ')" = "3" ] \
    || { "$CTL" list --json; fail "expected 3 panes (alpha split + beta)"; }

# --- D1: Ctrl+drag pane 1 header -> right edge of pane 2 -----------------
step "D1: ctrl+drag pane 1 header onto pane 2 right half"
xdotool keydown ctrl
xdotool mousemove "$HX1" "$HY1"
sleep 0.3                       # let egui see ctrl + the press
xdotool mousedown 1             # Sense::drag: latch fires on the press frame
sleep 0.2
xdotool keyup ctrl              # pane_drag latched; plain move is safe now
TX1=$((P2X + P2W * 85 / 100))   # dx 0.85 -> Right zone (center square is +-0.25)
xdotool mousemove "$TX1" "$MIDY"
sleep 0.6                       # several 50ms repaints: overlay must be up
kill -0 "$APP_PID" 2>/dev/null || fail "app died mid D1 drag"
scrot -o "$ROOT/d1.png"
export D1CAP="$ROOT/d1.png"
export D1PX=$((P2X + P2W * 3 / 4))   # center of the Right preview share
export D1MX=$((P2X + P2W / 4))       # mirrored point in pane 2's LEFT half
python3 - <<'PY'
import os, sys
from PIL import Image

BG = (40, 42, 54)          # dracula background
ACCENT = (189, 147, 249)   # dracula block_highlight

def mix(a, b, t):
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

img = Image.open(os.environ["D1CAP"]).convert("RGB")
px, mx, my = int(os.environ["D1PX"]), int(os.environ["D1MX"]), int(os.environ["MIDY"])
TOL = 3

def close(got, exp):
    return all(abs(g - e) <= TOL for g, e in zip(got, exp))

exp_fill = mix(BG, ACCENT, 0.22)
got = img.getpixel((px, my))
mir = img.getpixel((mx, my))
ok = close(got, exp_fill)
print(f"D1 preview fill: got {got} want {exp_fill} [{'OK' if ok else 'FAIL'}]")
ok2 = not close(mir, exp_fill)
print(f"D1 left half untouched: {mir} != {exp_fill} [{'OK' if ok2 else 'FAIL'}]")
sys.exit(0 if (ok and ok2) else 1)
PY
step "D1 overlay pixels ok; releasing"
xdotool mouseup 1
sleep 0.6                      # drop executes + dirty state saves
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
def shape(n):
    if "Pane" in n:
        return n["Pane"]["id"]
    s = n["Split"]
    return {"axis": s["axis"], "ratio": round(s["ratio"], 3),
            "first": shape(s["first"]), "second": shape(s["second"])}
got = shape(w["tabs"][0]["root"])
ok = (got["axis"] == "v" and abs(got["ratio"] - 0.5) <= 0.01
      and got["first"] == 2 and got["second"] == 1)
print(f"D1 root after move: {got} [{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY

# --- D2: Ctrl+drag pane 1 header (now right) -> pane 2 CENTER = swap -----
step "D2: ctrl+drag pane 1 header onto pane 2 center (swap)"
xdotool keydown ctrl
xdotool mousemove $((P2X + P2W / 2)) "$HY1"   # pane 1 now owns the right half
sleep 0.3
xdotool mousedown 1
sleep 0.2
xdotool keyup ctrl
xdotool mousemove "$HX1" "$MIDY"              # exact center of pane 2
sleep 0.6
xdotool mouseup 1
sleep 0.6
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
def shape(n):
    if "Pane" in n:
        return n["Pane"]["id"]
    s = n["Split"]
    return {"axis": s["axis"], "ratio": round(s["ratio"], 3),
            "first": shape(s["first"]), "second": shape(s["second"])}
got = shape(w["tabs"][0]["root"])
ok = (got["axis"] == "v" and abs(got["ratio"] - 0.5) <= 0.01
      and got["first"] == 1 and got["second"] == 2)
print(f"D2 root after center swap: {got} [{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY

# --- D5: plain drag (no modifiers) pane 1 header -> pane 2 right edge ----
step "D5: plain-drag pane 1 header onto pane 2 right half"
xdotool mousemove "$HX1" "$HY1"
sleep 0.3                       # settle: press must not coalesce with the move
xdotool mousedown 1             # >=2 panes in tab: bare press latches the move
sleep 0.2
xdotool mousemove "$TX1" "$MIDY"
sleep 0.6
xdotool mouseup 1
sleep 0.6                       # drop executes + dirty state saves
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
def shape(n):
    if "Pane" in n:
        return n["Pane"]["id"]
    s = n["Split"]
    return {"axis": s["axis"], "ratio": round(s["ratio"], 3),
            "first": shape(s["first"]), "second": shape(s["second"])}
got = shape(w["tabs"][0]["root"])
ok = (got["axis"] == "v" and abs(got["ratio"] - 0.5) <= 0.01
      and got["first"] == 2 and got["second"] == 1)
print(f"D5 root after plain move: {got} [{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY

# --- D3: drag the active tab chip past the second chip -------------------
step "D3: chip drag reorders the tabs"
# Precondition: the D1/D2 pane moves must not have touched the tabs.
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
titles = [t["title"] for t in w["tabs"]]
ok = titles == ["alpha", "beta"] and w["active_tab"] == 0
print(f"D3 precondition: tabs {titles} active {w['active_tab']} "
      f"[{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY
# Park the pointer on bare chrome first (Xvfb rests it at the screen
# center = the split gutter) so the chip scan sees resting colors.
xdotool mousemove $((X + WIDTH * 30 / 100)) $((Y + 6))
sleep 0.5
scrot -o "$ROOT/d3scan.png"
export D3SCAN="$ROOT/d3scan.png"
CHIPX=$(python3 - <<'PY'
import os, sys
from PIL import Image

BG = (40, 42, 54)
ACCENT = (189, 147, 249)

def mix(a, b, t):
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

X, Y = int(os.environ["X"]), int(os.environ["Y"])
img = Image.open(os.environ["D3SCAN"]).convert("RGB")
fill = mix(BG, ACCENT, 0.18)          # active-chip fill (tab_active)
def close(p, e, tol=3):
    return all(abs(a - b) <= tol for a, b in zip(p, e))
# Chip row Y+4..Y+32 (underline at Y+30..32 excluded): scan for the
# active chip's fill; text glyphs interrupt runs but min/max still span it.
xs = [x for yy in range(Y + 6, Y + 30) for x in range(X, X + min(int(os.environ["WIDTH"]), 500))
      if close(img.getpixel((x, yy)), fill)]
if xs:
    left, right = min(xs), max(xs)
    w = right - left
    if 40 <= w <= 250:
        print((left + right) // 2)
        sys.exit(0)
    print(f"active-chip run width {w}px outside 40..250 (left {left}, right {right})",
          file=sys.stderr)
else:
    print("no active-chip fill found in the chip row", file=sys.stderr)
# Fallback: known chrome geometry - first chip starts at the row's left
# edge, chips are 44px+ wide; sanity-assert the row IS chrome at x=400.
chrome = mix(BG, (248, 248, 242), 0.045)
if not close(img.getpixel((X + 400, Y + 18)), chrome, 3):
    print(f"row sanity fail: pixel at (X+400,Y+18) is not bare chrome",
          file=sys.stderr)
    sys.exit(1)
print(X + 60)
PY
) || fail "chip scan failed"
CHIP1X=$CHIPX
CHIPY=$((Y + 18))
CHIP1_RIGHT=$((CHIP1X + 45))     # ~half the alpha chip's width
DROPX=$((CHIP1_RIGHT + 140))     # clearly right of chip 2's center
echo "chip1 press x=$CHIP1X drop x=$DROPX (row y=$CHIPY)"
xdotool mousemove "$CHIP1X" "$CHIPY"
sleep 0.3
xdotool mousedown 1
# Let the app process the PRESS in its own frame BEFORE moving: egui
# hit-tests at the pointer's latest per-frame position, so a press
# coalesced with the first move would latch the drag on the WRONG chip
# (or the bare chrome) - the classic flake this sleep eliminates.
sleep 0.3
for i in 1 2 3; do
    xdotool mousemove $((CHIP1X + (DROPX - CHIP1X) * i / 3)) "$CHIPY"
    sleep 0.15
done
sleep 0.3
xdotool mouseup 1
sleep 0.8
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
titles = [t["title"] for t in w["tabs"]]
active = w["active_tab"]
ok = titles == ["beta", "alpha"]
print(f"D3 tab order: {titles} [{'OK' if ok else 'FAIL'}]")
# The dragged tab must STAY the selected one (move_tab keeps active_tab
# tracking the anchor pane); its index is 1 after the reorder.
ok2 = 0 <= active < len(titles) and titles[active] == "alpha"
print(f"D3 dragged tab stays active: active_tab={active} -> "
      f"{titles[active] if 0 <= active < len(titles) else '?'} [{'OK' if ok2 else 'FAIL'}]")
sys.exit(0 if (ok and ok2) else 1)
PY

# --- shared: active-chip edge scan for the cross-tab drags (D6-D8) --------
# The dwell targets below need the ACTIVE chip's edges to aim at the
# OTHER chip. Chip x positions drift (the " [2]" pane-count suffix
# widens labels between tests), so each test re-scans fresh instead of
# caching D3's geometry. The pointer parks on bare chrome first so
# hover fills cannot skew the scan (Xvfb rests it on the split gutter).
scan_active_chip() {   # $1 = scrot tag; sets CHIP_L/CHIP_R; fails on a bad scan
    xdotool mousemove $((X + WIDTH * 30 / 100)) $((Y + 6))
    sleep 0.5
    scrot -o "$ROOT/chip-$1.png"
    local out
    out=$(CHIPSCAN="$ROOT/chip-$1.png" python3 - <<'PY'
import os, sys
from PIL import Image

BG = (40, 42, 54)
ACCENT = (189, 147, 249)

def mix(a, b, t):
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

X, Y = int(os.environ["X"]), int(os.environ["Y"])
img = Image.open(os.environ["CHIPSCAN"]).convert("RGB")
fill = mix(BG, ACCENT, 0.18)          # active-chip fill (tab_active)

def close(p, e, tol=3):
    return all(abs(a - b) <= tol for a, b in zip(p, e))

# Same rows as the D3 scan (underline band Y+30..32 excluded); text
# glyphs interrupt the fill runs but min/max still span the whole chip.
xs = [x for yy in range(Y + 6, Y + 30)
      for x in range(X, X + min(int(os.environ["WIDTH"]), 500))
      if close(img.getpixel((x, yy)), fill)]
if not xs:
    print("no active-chip fill found in the chip row", file=sys.stderr)
    sys.exit(1)
left, right = min(xs), max(xs)
if not 40 <= right - left <= 250:
    print(f"active-chip run width {right - left}px outside 40..250 "
          f"(left {left}, right {right})", file=sys.stderr)
    sys.exit(1)
print(left, right)
PY
    ) || return 1
    CHIP_L=${out% *}
    CHIP_R=${out#* }
}

# --- D6: cross-tab edge migration via chip dwell (alpha pane1 -> beta) ----
step "D6: cross-tab drag migrates a pane (dwell on chip)"
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
titles = [t["title"] for t in w["tabs"]]
ok = titles == ["beta", "alpha"] and w["active_tab"] == 1
print(f"D6 precondition: tabs {titles} active {w['active_tab']} "
      f"[{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY
scan_active_chip d6 || fail "active-chip scan failed before D6"
ALPHAL=$CHIP_L                    # alpha chip (2nd) left edge
BETAX=$((ALPHAL - 30))            # inside the beta chip (gap 5, width >=44)
xdotool keydown ctrl
xdotool mousemove $((P2X + P2W / 2)) "$HY1"  # pane 1 = alpha's right half
sleep 0.3                         # let egui see ctrl + the press
xdotool mousedown 1
sleep 0.2
xdotool keyup ctrl                # pane_drag latched; plain move is safe
xdotool mousemove "$BETAX" "$CHIPY"          # dwell on the beta chip
sleep 1.0                         # 0.4s dwell + repaint: beta fullscreen
xdotool mousemove $((AX + AW * 85 / 100)) "$MIDY"   # Right zone on pane 3
sleep 0.6                         # several 50ms repaints: overlay must be up
kill -0 "$APP_PID" 2>/dev/null || fail "app died mid D6 drag"
scrot -o "$ROOT/d6.png"
export D6CAP="$ROOT/d6.png"
export D6PX=$((AX + AW * 3 / 4))  # center of the Right preview share
export D6MX=$((AX + AW / 4))      # mirrored point in the left half
python3 - <<'PY'
import os, sys
from PIL import Image

BG = (40, 42, 54)          # dracula background
ACCENT = (189, 147, 249)   # dracula block_highlight

def mix(a, b, t):
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

img = Image.open(os.environ["D6CAP"]).convert("RGB")
px, mx, my = int(os.environ["D6PX"]), int(os.environ["D6MX"]), int(os.environ["MIDY"])
TOL = 3

def close(got, exp):
    return all(abs(g - e) <= TOL for g, e in zip(got, exp))

# Cross-tab drag: the SOURCE pane is not on screen anymore (its tab was
# dwelled away from) - the overlay must still paint the Right preview.
exp_fill = mix(BG, ACCENT, 0.22)
got = img.getpixel((px, my))
mir = img.getpixel((mx, my))
ok = close(got, exp_fill)
print(f"D6 cross-tab preview fill: got {got} want {exp_fill} "
      f"[{'OK' if ok else 'FAIL'}]")
ok2 = not close(mir, exp_fill)
print(f"D6 left half untouched: {mir} != {exp_fill} "
      f"[{'OK' if ok2 else 'FAIL'}]")
sys.exit(0 if (ok and ok2) else 1)
PY
step "D6 overlay pixels ok; releasing"
xdotool mouseup 1
sleep 0.8                         # cross-tab move + dirty state saves
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
def shape(n):
    if "Pane" in n:
        return n["Pane"]["id"]
    s = n["Split"]
    return {"axis": s["axis"], "ratio": round(s["ratio"], 3),
            "first": shape(s["first"]), "second": shape(s["second"])}
titles = [t["title"] for t in w["tabs"]]
beta = shape(w["tabs"][0]["root"])
alpha = shape(w["tabs"][1]["root"])
ok = (titles == ["beta", "alpha"] and w["active_tab"] == 0
      and beta["axis"] == "v" and abs(beta["ratio"] - 0.5) <= 0.01
      and beta["first"] == 3 and beta["second"] == 1
      and w["tabs"][0]["focused"] == 1
      and alpha == 2 and w["tabs"][1]["focused"] == 2)
print(f"D6 beta root {beta} focused {w['tabs'][0]['focused']}")
print(f"D6 alpha root {alpha} focused {w['tabs'][1]['focused']}")
print(f"D6 titles {titles} active {w['active_tab']} "
      f"[{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY

# --- D7: cross-tab CENTER swap via chip dwell (beta pane3 <-> alpha pane2) -
step "D7: cross-tab center swap via chip dwell"
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
titles = [t["title"] for t in w["tabs"]]
ok = titles == ["beta", "alpha"] and w["active_tab"] == 0
print(f"D7 precondition: tabs {titles} active {w['active_tab']} "
      f"[{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY
scan_active_chip d7 || fail "active-chip scan failed before D7"
BETAR=$CHIP_R                     # beta chip (1st) right edge
ALPHAX=$((BETAR + 35))            # inside the alpha chip (gap 5, width >=44)
xdotool keydown ctrl
xdotool mousemove $((AX + AW / 4)) "$HY1"    # pane 3 = beta v(3,1) left half
sleep 0.3
xdotool mousedown 1
sleep 0.2
xdotool keyup ctrl
xdotool mousemove "$ALPHAX" "$CHIPY"         # dwell on the alpha chip
sleep 1.0                         # dwell + repaint: alpha (pane 2) fullscreen
xdotool mousemove $((AX + AW / 2)) "$MIDY"   # pane 2 exact center = swap
sleep 0.6
xdotool mouseup 1
sleep 0.8
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
def shape(n):
    if "Pane" in n:
        return n["Pane"]["id"]
    s = n["Split"]
    return {"axis": s["axis"], "ratio": round(s["ratio"], 3),
            "first": shape(s["first"]), "second": shape(s["second"])}
beta = shape(w["tabs"][0]["root"])
alpha = shape(w["tabs"][1]["root"])
ok = (beta["axis"] == "v" and abs(beta["ratio"] - 0.5) <= 0.01
      and beta["first"] == 2 and beta["second"] == 1
      # beta keeps focusing pane 1 - it never moved; only the tab the
      # drag landed in (alpha) focuses the dragged pane.
      and w["tabs"][0]["focused"] == 1
      and alpha == 3 and w["tabs"][1]["focused"] == 3
      and w["active_tab"] == 1)
print(f"D7 beta root {beta} focused {w['tabs'][0]['focused']}")
print(f"D7 alpha root {alpha} focused {w['tabs'][1]['focused']} "
      f"active {w['active_tab']} [{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY

# --- D8: dragging the LAST pane away closes the emptied source tab --------
step "D8: emptied source tab closes (alpha pane3 -> beta pane2 left)"
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
titles = [t["title"] for t in w["tabs"]]
ok = titles == ["beta", "alpha"] and w["active_tab"] == 1
print(f"D8 precondition: tabs {titles} active {w['active_tab']} "
      f"[{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY
scan_active_chip d8 || fail "active-chip scan failed before D8"
ALPHAL=$CHIP_L                    # alpha chip (2nd) left edge
BETAX=$((ALPHAL - 30))            # inside the wider "beta [2]" chip
# alpha holds a SINGLE pane now: a bare header press would latch the
# OS-window drag instead - ctrl must still be down when the press lands.
xdotool keydown ctrl
xdotool mousemove $((AX + AW / 2)) "$HY1"    # fullscreen pane 3 header
sleep 0.3
xdotool mousedown 1
sleep 0.2
xdotool keyup ctrl
xdotool mousemove "$BETAX" "$CHIPY"          # dwell on the beta chip
sleep 1.0                         # switch: beta v(2,1), pane 2 left half
# Left zone on pane 2: pane 2 owns only the LEFT HALF of the area, so
# 10% of the whole area = rel dx 0.2 -> outside the +-0.25 center square.
xdotool mousemove $((AX + AW * 10 / 100)) "$MIDY"
sleep 0.6
xdotool mouseup 1
sleep 0.8
python3 - "$STATE" <<'PY'
import json, sys
w = json.load(open(sys.argv[1]))["windows"][0]
def shape(n):
    if "Pane" in n:
        return n["Pane"]["id"]
    s = n["Split"]
    return {"axis": s["axis"], "ratio": round(s["ratio"], 3),
            "first": shape(s["first"]), "second": shape(s["second"])}
titles = [t["title"] for t in w["tabs"]]
root = shape(w["tabs"][0]["root"])
inner = root["first"]             # pane 3 nested left of pane 2
ok = (titles == ["beta"] and len(w["tabs"]) == 1 and w["active_tab"] == 0
      and root["axis"] == "v" and abs(root["ratio"] - 0.5) <= 0.01
      and root["second"] == 1
      and inner["axis"] == "v" and abs(inner["ratio"] - 0.5) <= 0.01
      and inner["first"] == 3 and inner["second"] == 2
      and w["tabs"][0]["focused"] == 3)
print(f"D8 titles {titles} active {w['active_tab']}")
print(f"D8 root {root} focused {w['tabs'][0]['focused']} "
      f"[{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY

# --- D4: still alive, then quit cleanly -----------------------------------
step "D4: liveness + Ctrl+Shift+Q quit"
kill -0 "$APP_PID" 2>/dev/null || fail "app process died during the drags"
xdotool key --clearmodifiers ctrl+shift+q
GONE=""
for _ in $(seq 1 40); do
    if ! kill -0 "$APP_PID" 2>/dev/null \
        && [ -z "$(xdotool search --onlyvisible --name terminator-rust 2>/dev/null)" ]; then
        GONE=1; break
    fi
    sleep 0.25
done
[ -n "$GONE" ] || fail "app or window still alive after Ctrl+Shift+Q"
APP_PID=""   # cleanup must not kill a dead pid

echo "DRAGDROP E2E: PASS"
