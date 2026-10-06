#!/usr/bin/env bash
# Source after setup.sh; every check uses a real visible IBus candidate panel.
NATIVE_FAILURES=0
native_snapshot() {
    # Collect all boundaries and still exercise rename/commit/quit. The caller
    # must fail at the end if any native placement assertion failed.
    if ! python3 scripts/tests/ime/assertions.py snapshot "$1"; then
        NATIVE_FAILURES=$((NATIVE_FAILURES + 1))
    fi
}
step "M3c: delayed echo keeps committed text visible and moves native candidates"
"$CTL" send "$PANE" --text "python3 '$PWD/scripts/tests/ime/delayed_echo.py'"$'\n' >/dev/null
wait_capture DELAY_READY || fail "M3c: delayed echo fixture did not start"
sleep 0.2
xdotool type --delay 40 hanzi
xdotool key --clearmodifiers space
xdotool type --delay 40 ni
sleep 1.5
python3 scripts/tests/ime/assertions.py snapshot "$ROOT/delayed.png"
xdotool key --clearmodifiers Escape
sleep 0.3
scrot "$ROOT/delayed-cancel.png"
python3 scripts/tests/ime/assertions.py delayed "$ROOT/delayed.png" "$ROOT/delayed-cancel.png"

step "M3d: native candidates survive cursor moves, hidden cursor, wrap and resize"
xdotool type --delay 40 ni
sleep 0.3
for mode in h v b w; do
    "$CTL" send "$PANE" --text "$mode" >/dev/null
    sleep 0.4
    xdotool type a
    xdotool key BackSpace
    sleep 0.3
    native_snapshot "$ROOT/native-$mode.png"
done
xdotool windowsize "$WID" 900 600
sleep 0.4
"$CTL" send "$PANE" --text b >/dev/null
sleep 0.4
xdotool type a
xdotool key BackSpace
sleep 0.3
native_snapshot "$ROOT/native-resize.png"
xdotool windowsize "$WID" "$WIDTH" "$HEIGHT"
sleep 0.4
"$CTL" send "$PANE" --text t >/dev/null
sleep 0.4
xdotool type a
xdotool key BackSpace
sleep 0.3
native_snapshot "$ROOT/native-top.png"
xdotool key --clearmodifiers space
wait_capture '汉字你' || fail "M3d: commit after cursor changes was lost"
. scripts/tests/ime/wrap.sh
"$CTL" send "$PANE" --text q >/dev/null
sleep 0.4
