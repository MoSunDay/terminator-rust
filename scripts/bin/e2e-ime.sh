#!/usr/bin/env bash
# Headless e2e for pane IME input (XIM: ibus + libpinyin over Xvfb).
#   S1: shell is live (marker via the control socket, NOT xdotool - the
#       IME is active from frame one and eats plain latin keys).
#   M1: composing swallows keys - 'hanzi' typed with NO commit key never
#       reaches the pane (XFilterEvent consumed it).
#   M2: app preedit underline is visible outside the native candidate window;
#       the candidate window is adjacent to the input row without overlap.
#   M3: space commits the libpinyin default candidate (汉字) and the raw
#       UTF-8 lands in the pty.
#   M3b: preedit hides existing colored text; Escape restores that text.
#   M3c/M3d: delayed echo and visible native panel placement boundaries.
#   M3e: exact row end, wide-character wrap and last-row cancel/commit.
#   M4: tab-chip rename - double-click the chip, compose 'hanzi', commit;
#       the title must END in 汉字 (a raw first key would leave 'h' behind).
#   M5: pane-header rename - same via the header double-click, plus the
#       pane's FIRST key after the editor closed must compose too (the
#       re-created input context used to swallow it as raw latin).
#   M6: bare Shift toggles libpinyin to EN and an ASCII marker types
#       through the still-active IME.
#   M7: window stayed alive the whole time; Ctrl+Shift+Q quits; state
#       persisted.
# Usage: scripts/bin/e2e-ime.sh [--bundle DIRECTORY]
#       (needs Xvfb, xdotool, scrot, PIL, ibus,
#        ibus-libpinyin, dbus, x11-utils/xprop). E2E_KEEP=1 keeps scratch.
set -euo pipefail
cd "$(dirname "$0")/../.."

. scripts/tests/ime/setup.sh

# --- M1: composition swallows the keys -----------------------------------
step "M1: composing 'hanzi' never reaches the pane"
scrot "$ROOT/base.png"     # pre-composition baseline for M2
xdotool type --delay 150 'hanzi'
sleep 0.6
if cap | grep -q 'hanzi'; then
    ime_diag
    cap | tail -5
    fail "M1: 'hanzi' reached the pane - the XIM filter is NOT engaged"
fi
echo "M1 ok: composition swallowed the keystrokes"

# --- M2: distinguish app preedit from native panel highlight ink ------------
step "M2: preedit underline and native candidates stay separate"
python3 scripts/tests/ime/assertions.py snapshot "$ROOT/compose.png"

# --- M3: commit round-trip ------------------------------------------------
step "M3: space commits libpinyin default candidate"
xdotool key --clearmodifiers space
sleep 0.6
wait_capture '汉字' || { cap | tail -5; ime_diag; \
    fail "M3: committed candidate 汉字 never reached the pane"; }
echo "commit round-trip ok (汉字 in capture)"
xdotool key Return
sleep 0.4

step "M3b: preedit covers existing text and cancellation restores it"
"$CTL" send "$PANE" --text "printf '\033[2J\033[H\033[91mMMMMMMMMMMMM\033[0m\033[H'; IFS= read -r ime_line"$'\n' >/dev/null
wait_capture 'MMMMMMMMMMMM' || fail "M3b: colored text fixture missing"
sleep 0.4
scrot "$ROOT/overlap-base.png"
xdotool type --delay 150 'hanzi'
sleep 0.4
scrot "$ROOT/overlap-compose.png"
xdotool key --clearmodifiers Escape
sleep 0.4
scrot "$ROOT/overlap-cancel.png"
python3 - "$ROOT" <<'PY'
import sys
from PIL import Image
base, comp, cancel = [Image.open(f"{sys.argv[1]}/overlap-{s}.png").convert("RGB")
                      for s in ("base", "compose", "cancel")]
def blue(rgb):
    r, g, b = rgb
    return b - g >= 18 and r - g <= 5 and b >= 110
def red(rgb):
    r, g, b = rgb
    return r > g + 40 and r > b + 20
old = [(x, y) for y in range(base.height) for x in range(base.width) if red(base.getpixel((x, y)))]
assert old, "colored text fixture is missing"
ink = [(x, y) for y in range(max(0, min(y for x, y in old) - 4), max(y for x, y in old) + 6) for x in range(comp.width)
       if blue(comp.getpixel((x, y))) and not blue(base.getpixel((x, y)))]
if not ink:
    sys.exit("M3b: missing app preedit; overlap regression was not exercised")
left, right = min(x for x, y in ink), max(x for x, y in ink)
bottom = max(y for x, y in ink)
# Skip the cursor cell and the antialiased backing edges.
covered = [(x, y) for y in range(max(0, bottom - 24), bottom - 2)
           for x in range(left + 12, right - 2) if red(base.getpixel((x, y)))]
assert len(covered) >= 10, "fixture has too little existing ink under preedit"
assert not any(red(comp.getpixel(p)) for p in covered), "existing text bleeds through preedit"
assert all(red(cancel.getpixel(p)) for p in covered), "Escape did not restore existing text"
print(f"M3b ok: {len(covered)} existing ink pixels hidden and restored")
PY
xdotool key --clearmodifiers ctrl+c
sleep 0.4

. scripts/tests/ime/native.sh

# --- M4: rename editors compose from the FIRST key ------------------------
# The rename editors force focus with Response::request_focus(), whose IME
# interrupt makes egui-winit disable+re-enable IME; winit X11 answers that
# by DESTROYING and recreating the XIM input context, and a fresh context
# is never XSetICFocus-ed (only a window FocusIn does that), so the keys
# after it bypassed the input method and landed as raw latin. The editors
# now take focus through egui's non-interrupting Tab path (ui/mod.rs
# focus_rename_editor). A raw first key leaves 'h'/'n' in front of the
# committed CJK, so the suffix asserts below are the discriminator.
step "M4: tab rename - first key composes"
xdotool mousemove $((X + 30)) $((Y + 17))   # chip body, clear of the close zone
sleep 0.4
xdotool click --repeat 2 --delay 120 1      # opens the tab rename editor
sleep 1.2
xdotool type --delay 150 'hanzi'
sleep 0.5
xdotool key --clearmodifiers space          # commits 汉字 into the editor buffer
sleep 0.5
xdotool key Return
sleep 0.8
REN_TAB=$(python3 -c 'import json,os;print(json.load(open(os.path.join(os.environ["HOME"],".terminator-rust","state.json")))["windows"][0]["tabs"][0]["title"])')
echo "tab title after rename: '$REN_TAB'"
case "$REN_TAB" in
    *汉字) ;;
    *) fail "M4: tab title '$REN_TAB' does not end in 汉字 (first key leaked raw?)" ;;
esac
echo "M4 ok: tab rename committed 汉字 from the very first key"

# --- M5: the pane keeps its IME across a rename editor --------------------
step "M5: pane rename - first key composes, pane IME survives"
xdotool mousemove $((X + 200)) $((Y + 53))  # pane header title band
sleep 0.4
xdotool click --repeat 2 --delay 120 1      # opens the pane rename editor
sleep 1.2
xdotool type --delay 150 'niao'
sleep 0.5
xdotool key --clearmodifiers space
sleep 0.5
xdotool key Return
sleep 0.8
REN_PANE=$(python3 -c 'import json,os;print(json.load(open(os.path.join(os.environ["HOME"],".terminator-rust","state.json")))["windows"][0]["tabs"][0]["root"]["Pane"]["meta"]["manual_title"] or "")')
echo "pane title after rename: '$REN_PANE'"
case "$REN_PANE" in
    *鸟) ;;
    *) fail "M5: pane title '$REN_PANE' does not end in 鸟 (first key leaked raw?)" ;;
esac
# The second half of the bug hit the PANE, not the editor: a re-created
# input context is only re-focused by the next window FocusIn, so the FIRST
# key typed into the pane after the editor closed used to land as raw latin.
xdotool key --clearmodifiers ctrl+c         # clear the pending shell line
sleep 0.3
xdotool type --delay 150 'niao'
sleep 0.5
xdotool key --clearmodifiers space
sleep 0.6
wait_capture '鸟' || { cap | tail -5; ime_diag; \
    fail "M5: the pane's first key after the editor did not compose"; }
cap | grep -q 'niao' && { cap | tail -5; fail "M5: raw 'niao' reached the pane"; }
xdotool key Return                          # run it, leave a clean prompt
sleep 0.4
echo "M5 ok: pane rename + the pane's first key after it both composed 鸟"

# --- M6: English passthrough through the still-active IME -----------------
# libpinyin sits in Chinese mode after the CJK commits above; a bare Shift
# press/release toggles English passthrough and the engine stays active.
# Runs AFTER every composing phase so those need no mode juggling.
step "M6: bare Shift toggles libpinyin to EN, ASCII passes through"
MK="IME_OK_$((RANDOM % 30000))"
xdotool key --clearmodifiers Shift_L
sleep 0.4
xdotool type --delay 120 "echo $MK"
xdotool key Return
wait_capture "$MK" || { cap | tail -5; ime_diag; \
    fail "M6: ASCII marker '$MK' did not pass through the active IME"; }
echo "English passthrough ok ($MK in capture)"

# --- M7: alive throughout + quit -----------------------------------------
step "M7: liveness + Ctrl+Shift+Q quits"
xdotool search --name '^terminator-rust$' | grep -q . || fail "M7: window vanished mid-test"
kill -0 "$APP_PID" 2>/dev/null || fail "M7: app process died mid-test"
xdotool key --clearmodifiers ctrl+shift+q
for _ in $(seq 1 40); do
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.25
done
kill -0 "$APP_PID" 2>/dev/null && { "$CTL" list || true; \
    fail "M7: app still alive 10s after Ctrl+Shift+Q"; }
APP_PID=""
[ -s "$HOME/.terminator-rust/state.json" ] || fail "M7: state.json missing/empty after quit"
echo "app quit on Ctrl+Shift+Q, state persisted"

[ "${NATIVE_FAILURES:-0}" -eq 0 ] || fail "$NATIVE_FAILURES native candidate placement checks failed"
echo "e2e-ime: ALL GREEN (M1-M7, native panel, delayed echo and wrap/cancel)"
