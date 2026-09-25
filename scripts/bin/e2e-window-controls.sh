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
#   R6: the chrome close X opens a centered CONFIRM DIALOG (one click,
#       app + window stay alive). Legs: the popup is one solid centered
#       frame brighter than the dimmed backdrop (scrot diff); keys typed
#       while it is up never reach the pane (ctl capture, with a
#       pre-dialog negative control); Esc, a second click on the X cell
#       (the modal backdrop owns it) and the Cancel button dismiss it
#       without quitting; the Quit button quits every window (ROOT
#       Close, like Ctrl+Shift+Q).
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
# (opens the quit-confirm dialog, see R6), maximize W-33, minimize W-53,
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
CTL=target/debug/terminator-ctl
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

# dialog_probe <base.png> <new.png> <present|gone>: the quit-confirm
# dialog's pixel signature, from two scrots of the same window rect. Only
# BRIGHTENINGS count (new - base clamped at 0, channel-wise MAX, > 6): the
# modal backdrop merely DARKENS everything below it, so a pixel that got
# brighter can only be a dialog pixel - the popup fill (chrome_bg) IS
# brighter than the dimmed pane bg it covers. `present` asserts one
# centred popup: mask count band, width >= the dialog's own declared
# min content width (close_dialog set_min_width(330*m.s)), height band,
# centre within 30% of the window half-size, and - because the modal's
# width is NOT content-driven (the right-to-left button row right-aligns
# into the available width, which on the first frame is egui's 600px
# default_area_size; measured 600x118 at font 15) - that the mask FILLS
# its bbox with fill >= 90%: solid one-piece frame, so a second bright
# blob (hover plate) or a scattered region fails where a width band
# cannot. Writes the frame's absolute "right bottom" to $ROOT/r6-box.txt
# (the Quit anchor: right-60, bottom-26 = 12px inner margin + half of the
# 96x28 button, both measured); `gone` asserts the brightening collapsed
# (dismissed). Prints the measured numbers either way.
dialog_probe() {
    X="$X" Y="$Y" WIDTH="$WIDTH" HEIGHT="$HEIGHT" \
    E2E_DBASE="$1" E2E_DNEW="$2" E2E_DMODE="$3" \
    E2E_DBOX="$ROOT/r6-box.txt" python3 - <<'PY'
import os, sys
from PIL import Image, ImageChops

X, Y = int(os.environ["X"]), int(os.environ["Y"])
W, H = int(os.environ["WIDTH"]), int(os.environ["HEIGHT"])
mode = os.environ["E2E_DMODE"]

def win(path):
    return Image.open(path).convert("RGB").crop((X, Y, X + W, Y + H))

d = ImageChops.subtract(win(os.environ["E2E_DNEW"]), win(os.environ["E2E_DBASE"]))
r, g, b = d.split()
mx = ImageChops.lighter(ImageChops.lighter(r, g), b)   # channel-wise MAX
mask = mx.point(lambda v: 255 if v > 6 else 0)
count = mask.histogram()[255]
box = mask.getbbox()
area = W * H

if mode == "gone":
    ok = count < 300
    print(f"dialog dismissed: {count}px still brighter (limit 300), "
          f"residual bbox {box} [{'OK' if ok else 'FAIL'}]")
    sys.exit(0 if ok else 1)

if box is None:
    print("dialog absent: no pixel brightened inside the window",
          file=sys.stderr)
    sys.exit(1)
left, top, right, bottom = box          # right/bottom are exclusive
bw, bh = right - left, bottom - top
cx, cy = left + bw / 2.0, top + bh / 2.0
density = count / float(bw * bh)
ok = (5000 <= count <= area * 0.4 and bw >= 330 and bw <= 0.75 * W
      and 80 <= bh <= 240 and density >= 0.90
      and abs(cx - W / 2.0) <= 0.30 * W / 2.0
      and abs(cy - H / 2.0) <= 0.30 * H / 2.0)
print(f"dialog: {count}px brighter ({100.0 * count / area:.1f}% of the "
      f"window), bbox {bw}x{bh} solid {100.0 * density:.1f}%, centre "
      f"{cx:.0f},{cy:.0f} vs window centre {W / 2:.0f},{H / 2:.0f} "
      f"[{'OK' if ok else 'FAIL'}]")
if ok:
    with open(os.environ["E2E_DBOX"], "w") as f:
        f.write("%d %d\n" % (X + right, Y + bottom))
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

step "R6: chrome close X opens a centered confirm dialog"
launch_app app1b.log
XCELL_X=$((X+WIDTH-13))                 # chrome close X cell centre
XCELL_Y=$((Y+CHROME_Y))
NEUTRAL_X=$((X+30))                     # pane corner: far outside the popup,
NEUTRAL_Y=$((Y+HEIGHT-30))              # hover-free in every scrot
list=$("$CTL" --socket "$SOCK" list --json 2>/dev/null) \
    || fail "terminator-ctl list failed on the running app"
pane_id=$(printf '%s' "$list" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])' \
        2>/dev/null || true)
case "$pane_id" in
    ''|*[!0-9]*) fail "ctl list did not name a numeric pane id ('$pane_id')" ;;
esac

# Negative control: with NO dialog up the very same typing DOES reach the
# pane, so the leak assert further down is about the dialog's modality and
# not about a broken typing/capture path.
activate
xdotool type --delay 40 'echo PRE_DIALOG'
xdotool key Return
ok=""
for _ in $(seq 1 24); do
    if "$CTL" --socket "$SOCK" capture "$pane_id" --lines 40 2>/dev/null \
        | grep -q PRE_DIALOG; then ok=1; break; fi
    sleep 0.25
done
[ -n "$ok" ] || fail "negative control: typed keys never reached the pane"

park "$NEUTRAL_X" "$NEUTRAL_Y"
sleep 0.4
scrot -o "$ROOT/r6-base.png"            # baseline: no dialog, no hover delta

click_at "$XCELL_X" "$XCELL_Y"          # ONE click on the close X
sleep 0.6
kill -0 "$APP_PID" 2>/dev/null || fail "app died on the first close click"
ids=$(xdotool search --name '^terminator-rust$' 2>/dev/null || true)
[ -n "$ids" ] || fail "window vanished on the first close click"

park "$NEUTRAL_X" "$NEUTRAL_Y"          # drop the X cell's hover plate
sleep 0.4
scrot -o "$ROOT/r6-open.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-open.png" present \
    || fail "one close click did not open a centered confirm dialog"

step "R6: keys typed while the dialog is up never reach the pane"
activate
xdotool type --delay 40 'echo DIALOG_LEAK'
xdotool key Return
park "$NEUTRAL_X" "$NEUTRAL_Y"
sleep 0.4
scrot -o "$ROOT/r6-typed.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-typed.png" present \
    || fail "typing closed/moved the dialog (it must stay modal)"

step "R6: Esc dismisses the dialog, the typed marker never arrived"
activate
xdotool key --clearmodifiers Escape
sleep 0.6
park "$NEUTRAL_X" "$NEUTRAL_Y"
sleep 0.4
scrot -o "$ROOT/r6-esc.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-esc.png" gone \
    || fail "Esc did not dismiss the confirm dialog"
kill -0 "$APP_PID" 2>/dev/null || fail "app died on the Esc dismissal"
cap=$("$CTL" --socket "$SOCK" capture "$pane_id" --lines 40 2>/dev/null) \
    || fail "ctl capture failed after the dialog closed"
case "$cap" in
    *DIALOG_LEAK*)
        printf '%s\n' "$cap" | tail -5
        fail "keys typed behind the dialog leaked into the pane" ;;
esac
printf '%s' "$cap" | grep -q PRE_DIALOG \
    || { printf '%s\n' "$cap" | tail -5; fail "pane capture lost PRE_DIALOG"; }

step "R6: a second click on the close X only dismisses the dialog"
click_at "$XCELL_X" "$XCELL_Y"          # re-open (Esc dismissed it above)
park "$NEUTRAL_X" "$NEUTRAL_Y"
sleep 0.4
scrot -o "$ROOT/r6-reopen.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-reopen.png" present \
    || fail "the close X did not re-open the dialog"
click_at "$XCELL_X" "$XCELL_Y"          # the modal backdrop owns this click
park "$NEUTRAL_X" "$NEUTRAL_Y"          # drop the X cell's hover plate
sleep 0.5
scrot -o "$ROOT/r6-backdrop.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-backdrop.png" gone \
    || fail "a click on the X cell did not just dismiss the dialog"
kill -0 "$APP_PID" 2>/dev/null || fail "app died on the backdrop dismissal"

step "R6: the Cancel button dismisses without quitting"
click_at "$XCELL_X" "$XCELL_Y"          # re-open for the Cancel leg
park "$NEUTRAL_X" "$NEUTRAL_Y"
sleep 0.4
scrot -o "$ROOT/r6-cancel-open.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-cancel-open.png" present \
    || fail "the close X did not re-open the dialog for the Cancel leg"
read -r DBOX_R DBOX_B <"$ROOT/r6-box.txt" || fail "no dialog bbox recorded"
CANCEL_X=$((DBOX_R-60-96-6))            # left of Quit: 96px button + 6 spacing
CANCEL_Y=$((DBOX_B-26))
echo "dialog frame right/bottom ${DBOX_R},${DBOX_B}; Cancel centre ${CANCEL_X},${CANCEL_Y}"
click_at "$CANCEL_X" "$CANCEL_Y"
park "$NEUTRAL_X" "$NEUTRAL_Y"
sleep 0.5
scrot -o "$ROOT/r6-cancel.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-cancel.png" gone \
    || fail "the Cancel button did not dismiss the dialog"
kill -0 "$APP_PID" 2>/dev/null || fail "app died on the Cancel dismissal"

step "R6: the dialog's Quit button quits the whole app"
click_at "$XCELL_X" "$XCELL_Y"          # re-open for the confirm leg
park "$NEUTRAL_X" "$NEUTRAL_Y"
sleep 0.4
scrot -o "$ROOT/r6-confirm.png"
dialog_probe "$ROOT/r6-base.png" "$ROOT/r6-confirm.png" present \
    || fail "the close X did not re-open the dialog for the confirm leg"
read -r DBOX_R DBOX_B <"$ROOT/r6-box.txt" || fail "no dialog bbox recorded"
echo "dialog frame right/bottom ${DBOX_R},${DBOX_B}; Quit button centre $((DBOX_R-60)),$((DBOX_B-26))"
click_at $((DBOX_R-60)) $((DBOX_B-26))  # Quit (96x28, flush right)
wait_pid_gone "$APP_PID" 40 || fail "app survived the dialog's Quit button"
echo "R6: one close click opens the dialog, Esc + backdrop + Cancel dismiss it, Quit quits"
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
