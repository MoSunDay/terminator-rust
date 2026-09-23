#!/usr/bin/env bash
# e2e-lib.sh - helpers shared by the Xvfb-driven e2e scripts
# (e2e-dragdrop.sh, e2e-window-controls.sh). Source it right after the
# `set -euo pipefail` line, BEFORE the `cd` (so a relative $0 still
# resolves no matter which directory the script was invoked from):
#     source "$(dirname "$0")/e2e-lib.sh"
#
# The lib only DEFINES helpers - it never touches shell options, so the
# caller's `set -euo pipefail` keeps governing every call site. Contract:
#   - the caller sets `ROOT` (mktemp scratch dir) and, optionally,
#     `E2E_FAIL_LOG` (a glob such as "$ROOT/app*.log") whose tail `fail`
#     prints when a check dies;
#   - anything that DIFFERS between scripts (screen geometry, pane
#     shells, WM setup, app launch strategy) stays in the scripts and is
#     only ever a parameter here - never silently unified.

# --- pass/fail ------------------------------------------------------------

# fail <msg>: mark the run failed, tail $E2E_FAIL_LOG (if set, glob
# expanded, best-effort) and exit 1.
fail() {
    echo "FAIL: $*" >&2
    if [ -n "${E2E_FAIL_LOG:-}" ]; then
        tail -20 $E2E_FAIL_LOG 2>/dev/null || true
    fi
    exit 1
}

step() { echo "== $*"; }

# --- lifecycle ------------------------------------------------------------

# e2e_cleanup <pid>...: kill the given pids in order (empty entries are
# skipped), then honor E2E_KEEP (keep the scratch dir for debugging) or
# remove it. Intended body of each script's `cleanup` EXIT trap.
e2e_cleanup() {
    local pid
    for pid in "$@"; do
        [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    done
    sleep 0.3
    if [ "${E2E_KEEP:-0}" = "1" ]; then
        echo "(E2E_KEEP=1: scratch dir kept at $ROOT)"
    else
        rm -rf "$ROOT"
    fi
}

# e2e_sandbox <shell> <restore 0|1>: scratch HOME + XDG_RUNTIME_DIR,
# the control-socket path (SOCK/TERMINATOR_SOCK) and state.json path
# (STATE), plus the headless determinism pins. <restore>=1 additionally
# exports TERMINATOR_RESTORE for scripts that preset state.json (the
# default launch is a fresh tab).
e2e_sandbox() {
    export XDG_RUNTIME_DIR="$ROOT/runtime"
    export HOME="$ROOT/home"
    export SHELL="$1"
    SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
    export TERMINATOR_SOCK="$SOCK"
    STATE="$HOME/.terminator-rust/state.json"
    # No compositor in Xvfb: pin full opacity for deterministic pixels,
    # and pin hover fades / cursor blink to their end states.
    export TERMINATOR_OPAQUE=1
    export TERMINATOR_NO_MOTION=1
    if [ "$2" = "1" ]; then
        export TERMINATOR_RESTORE=1
    fi
    mkdir -p "$HOME/.terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"
}

# e2e_start_xvfb <WxHxD>: probe random free display numbers - a fixed or
# PID-derived number can collide with a stale /tmp/.X11-unix socket left
# by an earlier crashed run - and start Xvfb on the first one that
# answers xdpyinfo. Sets XVFB_PID + DISPLAY_N and exports DISPLAY (for
# xdotool + scrot).
e2e_start_xvfb() {
    DISPLAY_N=""
    for _ in $(seq 1 12); do
        N=$((100 + RANDOM % 880))
        [ -S "/tmp/.X11-unix/X$N" ] && continue
        Xvfb ":$N" -screen 0 "$1" & XVFB_PID=$!
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
}

# --- X window helpers -----------------------------------------------------

# wait_win <name-regex> [tries] -> echoes the newest matching window id.
wait_win() {
    local pat=$1 tries=${2:-40} ids
    for _ in $(seq 1 "$tries"); do
        ids=$(xdotool search --name "$pat" 2>/dev/null || true)
        if [ -n "$ids" ]; then
            echo "$ids" | tail -1
            return 0
        fi
        sleep 0.25
    done
    return 1
}

# wait_pid_gone <pid> [tries]: until `kill -0` stops working.
wait_pid_gone() {
    local pid=$1 tries=${2:-40}
    for _ in $(seq 1 "$tries"); do
        kill -0 "$pid" 2>/dev/null || return 0
        sleep 0.25
    done
    return 1
}

# geo: refresh the X/Y/WIDTH/HEIGHT shell vars from the live window.
geo() {
    local out
    out=$(xdotool getwindowgeometry --shell "$WID") \
        || fail "getwindowgeometry failed for '$WID'"
    eval "$out"
}

# wait_geo <desc> <predicate-fn>: poll the predicate against fresh
# geometry every 0.25s (40 tries = 10s), then re-check once after the
# final sleep so a last-instant WM transition is not missed.
wait_geo() {
    local desc=$1
    shift
    for _ in $(seq 1 40); do
        geo
        if "$@"; then return 0; fi
        sleep 0.25
    done
    geo
    if "$@"; then return 0; fi
    fail "geometry never became: $desc (now ${X},${Y} ${WIDTH}x${HEIGHT})"
}

# park <x> <y>: move the pointer and let an app frame hit-test it
# (egui hit-tests per frame at a ~50ms cadence; an instant click races
# it and can land on the previous frame's target).
park() { xdotool mousemove "$1" "$2"; sleep 0.3; }

# click_at <x> <y>: park, then a global XTest button-1 click (never
# --window: winit drops XSendEvent-delivered input).
click_at() { park "$1" "$2"; xdotool click 1; sleep 0.5; }

# activate: EWMH-activate the window. With a WM present, windowfocus
# alone does NOT deliver keyboard events; retry while the WM settles.
activate() {
    for _ in $(seq 1 20); do
        xdotool windowactivate "$WID" 2>/dev/null && return 0
        sleep 0.3
    done
    fail "windowactivate never succeeded for $WID"
}

# --- state.json helpers ---------------------------------------------------

tab_count() {
    python3 -c 'import json,sys
print(len(json.load(open(sys.argv[1]))["windows"][0]["tabs"]))' "$STATE"
}

active_tab() {
    python3 -c 'import json,sys
print(json.load(open(sys.argv[1]))["windows"][0]["active_tab"])' "$STATE"
}

# e2e_state <state.json>: run a python tree checker. The checker body
# arrives on THIS function's stdin (a quoted heredoc at the call site)
# and is appended to the shared preamble below, which every state.json
# assert in the e2e scripts duplicates: `w` = windows[0], `shape()` =
# the Split tree rendered as nested dicts with pane ids at the leaves.
e2e_state() {
    { cat <<'PY'
import json, os, sys
w = json.load(open(sys.argv[1]))["windows"][0]
def shape(n):
    if "Pane" in n:
        return n["Pane"]["id"]
    s = n["Split"]
    return {"axis": s["axis"], "ratio": round(s["ratio"], 3),
            "first": shape(s["first"]), "second": shape(s["second"])}
PY
      cat; } | python3 - "$1"
}

# e2e_assert_root <label> <first-id> <second-id>: windows[0]'s tab 0
# root must be a vertical 50/50 split holding exactly those two pane
# ids (the D1/D2/D5 drag-drop landing shapes).
e2e_assert_root() {
    E2E_LABEL=$1 E2E_FIRST=$2 E2E_SECOND=$3 e2e_state "$STATE" <<'PY'
got = shape(w["tabs"][0]["root"])
ok = (got["axis"] == "v" and abs(got["ratio"] - 0.5) <= 0.01
      and got["first"] == int(os.environ["E2E_FIRST"])
      and got["second"] == int(os.environ["E2E_SECOND"]))
print(f"{os.environ['E2E_LABEL']}: {got} [{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY
}

# e2e_assert_tabs <label> <title0> <title1> <active>: windows[0] must
# hold exactly those two tab titles with active_tab == <active>.
e2e_assert_tabs() {
    E2E_LABEL=$1 E2E_T0=$2 E2E_T1=$3 E2E_ACTIVE=$4 e2e_state "$STATE" <<'PY'
titles = [t["title"] for t in w["tabs"]]
ok = (titles == [os.environ["E2E_T0"], os.environ["E2E_T1"]]
      and w["active_tab"] == int(os.environ["E2E_ACTIVE"]))
print(f"{os.environ['E2E_LABEL']}: tabs {titles} active {w['active_tab']} "
      f"[{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY
}

# e2e_preset_tabs <state.json> <count>: preset <count> single-pane tabs
# "tabname-01".. with fixed-width titles (uniform ~106px chips for the
# overflow tests; pane/meta shape mirrors e2e-dragdrop.sh's preset -
# ids are remapped in preorder on load anyway).
e2e_preset_tabs() {
    python3 - "$1" "$2" <<'PY'
import json, sys
meta = {"kind": "Local", "manual_title": None, "bg": None,
        "transparency": 0.0, "degraded": False}
tabs = []
for i in range(1, int(sys.argv[2]) + 1):
    pid = 10 + i
    tabs.append({"title": "tabname-%02d" % i, "focused": pid,
                 "root": {"Pane": {"id": pid, "meta": meta}}})
state = {"theme": "dracula",
         "settings": {"split_axis": "v", "split_ratio": 0.5},
         "windows": [{"id": 1, "active_tab": 0, "tabs": tabs}]}
with open(sys.argv[1], "w") as f:
    json.dump(state, f, indent=2)
PY
}

# --- scrot + PIL pixel helpers -------------------------------------------

# e2e_chip_edges: scan the chip row of $CHIPSCAN for the ACTIVE chip's
# fill mix(bg, accent, 0.18) and print "<left> <right>" edges. Needs the
# window geometry exported as X/Y/WIDTH. Rows Y+6..Y+30 (underline band
# Y+30..32 excluded); text glyphs interrupt the fill runs but min/max
# still span the whole chip. Exits 1 when no plausible run is found
# (width outside 40..250) - callers decide whether that is fatal or
# falls back to known chrome geometry.
e2e_chip_edges() {
    python3 - <<'PY'
import os, sys
from PIL import Image

BG = (40, 42, 54)          # dracula background
ACCENT = (189, 147, 249)   # dracula block_highlight

def mix(a, b, t):
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

X, Y = int(os.environ["X"]), int(os.environ["Y"])
img = Image.open(os.environ["CHIPSCAN"]).convert("RGB")
fill = mix(BG, ACCENT, 0.18)          # active-chip fill (tab_active)

def close(p, e, tol=3):
    return all(abs(a - b) <= tol for a, b in zip(p, e))

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
}

# e2e_overlay_fill_check <label>: mid-drag dropzone overlay probe.
# Reads E2E_OLCAP (scrot path), E2E_OLPX (a point inside the preview
# share), E2E_OLMX (the mirrored point that must stay untouched) and
# the exported MIDY; asserts the fill mix(bg, accent, 0.22) at
# (E2E_OLPX, MIDY) and NOT at (E2E_OLMX, MIDY).
e2e_overlay_fill_check() {
    E2E_OLTAG=$1 python3 - <<'PY'
import os, sys
from PIL import Image

BG = (40, 42, 54)          # dracula background
ACCENT = (189, 147, 249)   # dracula block_highlight

def mix(a, b, t):
    return tuple(int(round(a[i] + t * (b[i] - a[i]))) for i in range(3))

img = Image.open(os.environ["E2E_OLCAP"]).convert("RGB")
px, mx, my = int(os.environ["E2E_OLPX"]), int(os.environ["E2E_OLMX"]), int(os.environ["MIDY"])
TOL = 3

def close(got, exp):
    return all(abs(g - e) <= TOL for g, e in zip(got, exp))

tag = os.environ["E2E_OLTAG"]
exp_fill = mix(BG, ACCENT, 0.22)
got = img.getpixel((px, my))
mir = img.getpixel((mx, my))
ok = close(got, exp_fill)
print(f"{tag} preview fill: got {got} want {exp_fill} [{'OK' if ok else 'FAIL'}]")
ok2 = not close(mir, exp_fill)
print(f"{tag} left half untouched: {mir} != {exp_fill} [{'OK' if ok2 else 'FAIL'}]")
sys.exit(0 if (ok and ok2) else 1)
PY
}
