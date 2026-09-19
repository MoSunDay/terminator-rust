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
STATE_REL="terminator-rust/state.json"
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

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/bash
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
STATE="$XDG_CONFIG_HOME/$STATE_REL"
# No compositor in Xvfb: pin full opacity for deterministic pixels, and
# pin hover fades / cursor blink to their end states.
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

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
# Single-row chrome = 3 + chip 24 + 3 = 30px; pane area below it. A
# v-split at 0.5 with a 6px divider: left w = (W-6)/2, right the rest.
CHROME_H=30; GUT=6
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
# Chip row Y+3..Y+27 (underline at Y+25..27 excluded): scan for the
# active chip's fill; text glyphs interrupt runs but min/max still span it.
xs = [x for yy in range(Y + 5, Y + 24) for x in range(X, X + min(int(os.environ["WIDTH"]), 500))
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
if not close(img.getpixel((X + 400, Y + 15)), chrome, 3):
    print(f"row sanity fail: pixel at (X+400,Y+15) is not bare chrome",
          file=sys.stderr)
    sys.exit(1)
print(X + 60)
PY
) || fail "chip scan failed"
CHIP1X=$CHIPX
CHIPY=$((Y + 15))
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
