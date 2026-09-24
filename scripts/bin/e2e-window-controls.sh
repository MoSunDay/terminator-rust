#!/usr/bin/env bash
# Headless e2e: window controls under a real EWMH WM (openbox on Xvfb).
#   R1: edge drags resize the window APP-DRIVEN: per-frame
#       ViewportCommands steered by an X-polled pointer (the WM only
#       applies geometry). R1a: an east drag lands the exact pointer
#       width. R1b: after release the geometry is FROZEN - pointer
#       wander must not follow (the original bug). R1c: a west drag
#       moves the origin, the direction whose per-frame XMoveWindow
#       used to break the WM pointer grab.
#   R2: the chrome maximize/restore button toggles ViewportCommand::
#       Maximized (openbox has no panel -> maximize == 1400x900) and the
#       second press restores the pre-maximize size.
#   R3: double-clicking BARE chrome (between the chips and the fixed
#       right-hand buttons) toggles maximize / restore the same way.
#   R4: the minimize glyph is a dash CENTRED on its cell (pixel probe:
#       a dash sitting low reads as '_'), and the button iconifies the
#       window: no longer --onlyvisible while the app stays alive;
#       windowactivate restores.
#   R5: Ctrl+Shift+Q quits the app cleanly.
#   R6: the chrome close X needs a SECOND confirming click: the first
#       click only arms (app + window stay alive), a click anywhere
#       else disarms, and a fresh second click quits the whole app.
#   S1: at scroll 0 (wheel-up normalised) the chip slot at
#       (chip-right - 40px, safely left of the per-chip close button)
#       holds an EARLY tab, not the last one.
#   S2: wheel notches (48px each) scroll the chip row to its end so the
#       LAST tab lands in that slot; no tab is lost on the way.
#   S3: with the chip row overflowing, the trailing '+' button PARKS
#       flush left of the pinned edge cells and still spawns +
#       activates tab 13.
#   S4: with only TWO chips (no overflow) the trailing +/split group
#       FOLLOWS the chips: '+' at chips-right + 8px spawns + activates
#       tab 3, while the old parked slot at W-157 is bare chrome (a
#       click there must not add a tab).
#   S5: Ctrl+Shift+Q quits the app cleanly.
#
# Chrome geometry contract under test (single ~36px row): the EDGE
# CELLS (close/max/min/insp/zoom) stay PINNED at the far right - 16px
# wide / 4px apart / rightmost 5px from the edge, close center at W-13
# (double-confirm quit, see R6), maximize W-33, minimize W-53,
# inspector W-73, zoom W-93. The trailing group ('+', split-v,
# split-h) no longer pins: it FOLLOWS the chips (8px right of the last
# chip) and only PARKS flush left of the edge cells when the chips
# overflow the strip (GROUP_PARK: parked '+' center at W-157, chip
# strip bound at W-173). The chip-row center y sits at Y+18. Vertical
# wheel = horizontal chip scroll, one notch = 48px.
#
# Usage: scripts/bin/e2e-window-controls.sh  (repo root; needs Xvfb +
#        xdotool + openbox).  E2E_KEEP=1 keeps the scratch dir.
set -euo pipefail
source "$(dirname "$0")/e2e-lib.sh"
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-wc-XXXXXX)
APP=target/debug/terminator-rust
DISPLAY_N=""                  # probed by e2e_start_xvfb
XVFB_PID=""
OPENBOX_PID=""
APP_PID=""
WID=""
CHROME_Y=18                   # chip-row center y offset inside the window
E2E_FAIL_LOG="$ROOT/app*.log"

cleanup() { e2e_cleanup "$APP_PID" "$OPENBOX_PID" "$XVFB_PID"; }
trap cleanup EXIT

# --- script-specific geometry predicates ---------------------------------

is_maximized()     { [ "$WIDTH" -eq 1400 ] && [ "$HEIGHT" -eq 900 ]; }
is_not_maximized() { [ "$WIDTH" -lt 1400 ]; }

# near_target: un-maximized and within +/-10px of $W_TARGET (the width
# captured right after the R1 resize).
near_target() {
    is_not_maximized || return 1
    local d=$((WIDTH - W_TARGET))
    [ "$d" -le 10 ] && [ "$((0 - d))" -le 10 ]
}

# min_glyph_check <scrot.png>: the MINIMIZE cell must hold a horizontal
# dash whose ink centroid sits on the chrome row's centre (the glyph used
# to be drawn 4px low, which reads as '_'). Theme-agnostic: the band's
# modal luminance is the background, ink weight = lum - bg - 25 (a hover
# plate or a hairline stays below the floor), so no palette constants.
min_glyph_check() {
    E2E_MCAP="$1" E2E_MCX=$((X + WIDTH - 53)) E2E_MCY=$((Y + CHROME_Y)) \
    python3 - <<'PY'
import os, sys
from collections import Counter
from PIL import Image

img = Image.open(os.environ["E2E_MCAP"]).convert("RGB")
cx, cy = int(os.environ["E2E_MCX"]), int(os.environ["E2E_MCY"])

def lum(px):
    return 0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2]

band = [(x, y) for y in range(cy - 12, cy + 13)
        for x in range(cx - 8, cx + 9)]
vals = {p: lum(img.getpixel(p)) for p in band}
bg = Counter(round(v) for v in vals.values()).most_common(1)[0][0]
ink = {p: v - bg - 25.0 for p, v in vals.items() if v - bg - 25.0 > 0.0}
tot = sum(ink.values())
if tot <= 0:
    print(f"no minimize ink around {cx},{cy} (band bg lum {bg})",
          file=sys.stderr)
    sys.exit(1)
centroid = sum(w * p[1] for p, w in ink.items()) / tot
xs = [p[0] for p in ink]
width = max(xs) - min(xs) + 1
ok = abs(centroid - cy) <= 1.0 and width >= 6
print(f"min glyph: ink centroid y {centroid:.1f} (cell centre {cy}), "
      f"width {width}px [{'OK' if ok else 'FAIL'}]")
sys.exit(0 if ok else 1)
PY
}

# launch_app <log>: start the app on the running Xvfb, wait for the
# control socket + window, EWMH-activate it, snapshot the geometry.
launch_app() {
    local log=$1
    rm -f "$SOCK"    # start() reclaims stale sockets; keep the wait honest
    env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid nohup "$APP" </dev/null \
        >"$ROOT/$log" 2>&1 & APP_PID=$!
    for _ in $(seq 1 40); do
        kill -0 "$APP_PID" 2>/dev/null \
            || { tail "$ROOT/$log"; fail "app died at startup"; }
        [ -S "$SOCK" ] && break
        sleep 0.25
    done
    [ -S "$SOCK" ] || { tail "$ROOT/$log"; fail "control socket never appeared"; }
    WID=$(wait_win '^terminator-rust$') \
        || { tail "$ROOT/$log"; fail "app window not found"; }
    activate
    sleep 1
    geo
    kill -0 "$APP_PID" 2>/dev/null || fail "app died during startup"
    echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"
}

# --- build + sandbox ------------------------------------------------------
cargo build -p app -p ctl --bins >/dev/null
# plain sh panes: no OSC title churn, window names stay deterministic;
# session restore ON: the part-2 presets below must load.
e2e_sandbox /bin/sh 1

# --- Xvfb + openbox -------------------------------------------------------
step "launch Xvfb + openbox"
e2e_start_xvfb 1400x900x24

# A real EWMH WM: keyboard focus needs windowactivate, and maximize /
# minimize / move-resize land via EWMH properties.
env DISPLAY="$DISPLAY_N" setsid openbox >/dev/null 2>&1 & OPENBOX_PID=$!
sleep 1
kill -0 "$OPENBOX_PID" 2>/dev/null || fail "openbox failed to start"

# --- part 1: window controls (fresh state, single "shell" tab) ------------
step "part 1: window controls on a fresh state"
launch_app app1.log

step "R1a: right-edge drag resizes to the exact pointer width"
geo
W0=$WIDTH; X0=$X
park $((X+WIDTH-2)) $((Y+HEIGHT/2))     # inside the 6px edge zone
xdotool mousedown 1
sleep 0.3     # press and first move must not share a frame (anchor race)
for i in 1 2 3 4 5 6 7 8; do
    xdotool mousemove $((X0+W0-2+i*10)) $((Y+HEIGHT/2)); sleep 0.06
done
xdotool mouseup 1
sleep 0.6
geo
[ "$WIDTH" -eq $((W0+80)) ] \
    || fail "edge drag: ${W0} -> ${WIDTH} (want exactly ${W0}+80)"
echo "R1a: width ${W0} -> ${WIDTH} exactly"

step "R1b: after release the window stops following the pointer"
for px in $((X0+40)) $((X0+W0-2)) $((X0+W0+40)) $((X0+WIDTH/2)); do
    xdotool mousemove "$px" $((Y+HEIGHT/2)); sleep 0.3
done
geo
[ "$WIDTH" -eq $((W0+80)) ] && [ "$X" -eq "$X0" ] \
    || fail "post-release follow: ${X},${Y} ${WIDTH}x${HEIGHT}"
echo "R1b: frozen at ${X},${Y} ${WIDTH}x${HEIGHT}"

step "R1c: west-edge drag moves the origin, width grows"
park $X0 $((Y+HEIGHT/2))                # first pixel column = west strip
xdotool mousedown 1
sleep 0.3
for i in 1 2 3 4 5 6; do
    xdotool mousemove $((X0-i*10)) $((Y+HEIGHT/2)); sleep 0.06
done
xdotool mouseup 1
sleep 0.6
geo
[ "$X" -eq $((X0-60)) ] && [ "$WIDTH" -eq $((W0+140)) ] \
    || fail "west drag: ${X},${WIDTH} (want $((X0-60)),$((W0+140)))"
echo "R1c: origin ${X0} -> ${X}, width -> ${WIDTH}"
W_TARGET=$WIDTH

step "R2: maximize button toggles 1400x900 and restores"
click_at $((X+WIDTH-33)) $((Y+CHROME_Y))    # maximize/restore
wait_geo "maximized (1400x900)" is_maximized
click_at $((X+WIDTH-33)) $((Y+CHROME_Y))
wait_geo "restored to ~${W_TARGET}px" near_target
echo "R2: toggled to 1400x900 and back to ${WIDTH}"

step "R3: double-click on bare chrome toggles maximize/restore"
park $((X+WIDTH*60/100)) $((Y+CHROME_Y))    # bare chrome right of the chip
xdotool click --repeat 2 --delay 150 1
sleep 0.5
wait_geo "maximized via double-click" is_maximized
park $((X+WIDTH*60/100)) $((Y+CHROME_Y))
xdotool click --repeat 2 --delay 150 1
sleep 0.5
wait_geo "restored via double-click" is_not_maximized
echo "R3: double-click toggle ok (${WIDTH}x${HEIGHT})"

step "R4: minimize glyph is a centred dash, then the button iconifies"
park $((X+WIDTH*60/100)) $((Y+CHROME_Y))    # bare chrome: no hover plate
sleep 0.5                                   # settle the parked frame
scrot -o "$ROOT/min-glyph.png"
min_glyph_check "$ROOT/min-glyph.png" \
    || fail "minimize glyph is not a centred dash (see $ROOT/min-glyph.png)"
click_at $((X+WIDTH-53)) $((Y+CHROME_Y))    # minimize
sleep 0.5
ids=$(xdotool search --onlyvisible --name '^terminator-rust$' 2>/dev/null || true)
[ -z "$ids" ] || fail "window still visible after minimize: $ids"
kill -0 "$APP_PID" 2>/dev/null || fail "app died on minimize"
echo "R4: glyph centred, minimized (iconic, app alive)"
activate
vis=""
for _ in $(seq 1 40); do
    ids=$(xdotool search --onlyvisible --name '^terminator-rust$' 2>/dev/null || true)
    if [ -n "$ids" ]; then vis=1; break; fi
    sleep 0.25
done
[ -n "$vis" ] || fail "window never became visible again after windowactivate"
geo
echo "R4: restored at ${X},${Y} ${WIDTH}x${HEIGHT}"

step "R5: Ctrl+Shift+Q quits the app"
activate
xdotool key --clearmodifiers ctrl+shift+q
wait_pid_gone "$APP_PID" 40 || fail "app survived Ctrl+Shift+Q (part 1)"
echo "app exited cleanly"
APP_PID=""
WID=""
sleep 0.5

step "R6: chrome close X needs a second confirming click"
launch_app app1b.log
click_at $((X+WIDTH-13)) $((Y+CHROME_Y))    # close: first click only ARMS
sleep 0.6
kill -0 "$APP_PID" 2>/dev/null || fail "app died on the arming close click"
ids=$(xdotool search --name '^terminator-rust$' 2>/dev/null || true)
[ -n "$ids" ] || fail "window vanished on the arming close click"
click_at $((X+WIDTH*60/100)) $((Y+CHROME_Y))   # bare chrome click disarms
sleep 0.6
kill -0 "$APP_PID" 2>/dev/null || fail "app died after the cancelling click"
click_at $((X+WIDTH-13)) $((Y+CHROME_Y))    # re-arm
sleep 0.3
click_at $((X+WIDTH-13)) $((Y+CHROME_Y))    # confirming click quits all
wait_pid_gone "$APP_PID" 40 || fail "app survived the confirmed close click"
echo "R6: one close click arms, the second quits"
APP_PID=""
WID=""
sleep 0.5

# --- part 2: chip-row overflow scrolling ----------------------------------
step "part 2: preset 12 overflowing tabs and relaunch"
# 12 single-pane tabs "tabname-01".."tabname-12": fixed-width titles ->
# uniform ~106px chips, 12 * 106 = 1272px > the ~1027px chip area of a
# 1200px window. Pane/meta shape mirrors e2e-dragdrop.sh's preset (ids
# are remapped in preorder on load anyway).
e2e_preset_tabs "$STATE" 12

launch_app app2.log
sleep 1   # let the 12 preset shells settle
geo
[ "$WIDTH" -eq 1200 ] && [ "$HEIGHT" -eq 800 ] \
    || fail "expected the default 1200x800 window, got ${WIDTH}x${HEIGHT}"

# 12 preset tabs: the LAST one is index 11. Normalise the strip to scroll 0
# first (the auto-follow could have scrolled it into view at preset time).
step "S1: at scroll 0, the chip at right-60 is an EARLY tab"
park $((X+WIDTH/2)) $((Y+CHROME_Y))
for _ in $(seq 1 10); do xdotool click 4; sleep 0.12; done   # wheel up = scroll 0
sleep 0.5
click_at $((X+WIDTH-173-40)) $((Y+CHROME_Y))
A=""
for _ in $(seq 1 20); do
    A=$(active_tab)
    [ "$A" != "11" ] && break
    sleep 0.25
done
[ "$A" != "11" ] \
    || fail "right-40 slot already held the LAST tab at scroll 0"
[ "$(tab_count)" -eq 12 ] \
    || fail "the S1 click hit a close button (tabs=$(tab_count))"
echo "S1: slot holds tab index ${A}"

step "S2: 10 wheel notches (48px each) scroll the chip row to its end"
park $((X+WIDTH/2)) $((Y+CHROME_Y))
for _ in $(seq 1 10); do xdotool click 5; sleep 0.12; done
sleep 0.5
click_at $((X+WIDTH-173-40)) $((Y+CHROME_Y))
ok=""
for _ in $(seq 1 20); do
    if [ "$(active_tab)" -eq 11 ]; then ok=1; break; fi
    sleep 0.25
done
[ -n "$ok" ] \
    || fail "scroll did not bring the LAST tab into the right-40 slot (active=$(active_tab))"
[ "$(tab_count)" -eq 12 ] || fail "scrolling lost tabs: count=$(tab_count)"
echo "S2: last tab now under the slot; 12 tabs intact"

step "S3: overflow parks the trailing '+' left of the edge cells"
click_at $((X+WIDTH-157)) $((Y+CHROME_Y))   # parked '+' (new tab)
ok=""
for _ in $(seq 1 20); do
    if [ "$(tab_count)" -eq 13 ] && [ "$(active_tab)" -eq 12 ]; then ok=1; break; fi
    sleep 0.25
done
[ -n "$ok" ] \
    || fail "'+' did not add+activate tab 13 (tabs=$(tab_count) active=$(active_tab))"
echo "S3: 13 tabs, active_tab=12"

step "S4: the trailing group FOLLOWS the chips when they fit"
activate
xdotool key --clearmodifiers ctrl+shift+q
wait_pid_gone "$APP_PID" 40 || fail "app survived Ctrl+Shift+Q (part 2)"
APP_PID=""
WID=""
sleep 0.5

# Two short chips fit easily, so the group rides 8px right of chip 2:
# the chip label renders at the terminal font (15pt), so each
# "tabname-NN" chip is 124px -> 124 + 5 + 124 = 253px of chips, + 8px
# gap, + 8px half-icon puts the '+' center at X+269. The OLD parked slot
# (W-157) is bare chrome now - a click there must be a no-op.
# 2 single-pane tabs "tabname-01"/"tabname-02": same PTab shape as the
# 12-tab preset above (ids are remapped in preorder on load anyway).
e2e_preset_tabs "$STATE" 2

launch_app app3.log
sleep 1   # let the two preset shells settle
geo
[ "$(tab_count)" -eq 2 ] \
    || fail "S4 preset did not restore 2 tabs (tabs=$(tab_count))"

click_at $((X+269)) $((Y+CHROME_Y))    # '+' following the second chip
ok=""
for _ in $(seq 1 20); do
    if [ "$(tab_count)" -eq 3 ] && [ "$(active_tab)" -eq 2 ]; then ok=1; break; fi
    sleep 0.25
done
[ -n "$ok" ] \
    || fail "trailing group did not follow the chips: '+' at X+269 did not add+activate tab 3 (tabs=$(tab_count) active=$(active_tab))"

click_at $((X+WIDTH-157)) $((Y+CHROME_Y))   # OLD parked '+' slot = bare chrome
sleep 1
[ "$(tab_count)" -eq 3 ] \
    || fail "old parked slot still hosts the + button (tabs=$(tab_count))"
echo "S4: group follows the chips ('+' at X+269 works, W-157 slot inert)"

step "S5: Ctrl+Shift+Q quits the app"
activate
xdotool key --clearmodifiers ctrl+shift+q
wait_pid_gone "$APP_PID" 40 || fail "app survived Ctrl+Shift+Q (part 2)"
echo "app exited cleanly"

echo "WINDOW-CONTROLS E2E: PASS"
