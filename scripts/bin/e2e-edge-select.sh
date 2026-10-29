#!/usr/bin/env bash
# Isolated X11: drag-selection that runs into the window BORDER STRIPS.
# A pane flush with the window edge has its last columns/rows under the
# 6px/14px resize strips: the drag must keep extending to the border
# cell, and the release INSIDE the strip must end the grab - a swallowed
# release strands a phantom grab that keeps re-anchoring the selection to
# the bare pointer, so the copy afterwards returns the wrong text.
#
# Rows are numbered and padded to the measured grid width, so the copy is
# asserted cell-exact against `ctl capture`: a frozen endpoint loses the
# last columns, a phantom grab returns the wrong rows.
set -euo pipefail
ROOT=$(mktemp -d)
E2E_FAIL_LOG="$ROOT/app.log"
source "$(dirname "$0")/e2e-lib.sh"
cd "$(dirname "$0")/../.."
APP="${E2E_APP:-${CARGO_TARGET_DIR:-target}/release/terminator-rust}"
CTL="${CARGO_TARGET_DIR:-target}/release/terminator-ctl"
APP=$(realpath "$APP"); CTL=$(realpath "$CTL")
APP_PID=""; XVFB_PID=""
cleanup() { e2e_cleanup "$APP_PID" "$XVFB_PID"; }
trap cleanup EXIT
e2e_sandbox /bin/bash 0
e2e_start_xvfb 1200x800x24
setsid "$APP" >"$ROOT/app.log" 2>&1 < /dev/null & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep .25; done
WID=$(wait_win terminator-rust)
xdotool windowmap --sync "$WID"
sleep .5
xdotool windowfocus "$WID"
eval "$(xdotool getwindowgeometry --shell "$WID")"
ID=$("$CTL" list | awk 'NR==2 {print $1}')

wait_clipboard() {
    for _ in $(seq 1 50); do
        [ "$(xclip -o -selection clipboard 2>/dev/null)" = "$1" ] && return 0
        sleep .1
    done
    return 1
}

# Trailing-whitespace trim per line (a selection to the last grid column
# may carry padded spaces; `ctl capture` trims them too).
norm() { python3 -c 'import sys; print("\n".join(l.rstrip() for l in sys.stdin.read().splitlines()))'; }

# Drag with the press/motion settles the egui dragdrop e2e taught us
# (the press frame must not coalesce with the first motion).
drag_select() { # <x0> <y0> <x1> <y1>
    xdotool mousemove "$1" "$2"
    sleep .3
    xdotool mousedown 1
    sleep .25
    local n=5 i
    for i in $(seq 1 "$n"); do
        xdotool mousemove \
            $(( ($1 * ($n - i) + $3 * i) / $n )) \
            $(( ($2 * ($n - i) + $4 * i) / $n ))
        sleep .05
    done
    xdotool mousemove "$3" "$4"
    sleep .3
    xdotool mouseup 1
    sleep .2
}

copy_selection() {
    xdotool mousemove $((X + 300)) $((Y + 300))   # bare pointer: must NOT move it
    sleep .6
    xdotool key ctrl+shift+c
    sleep .5
    xclip -o -selection clipboard 2>/dev/null
}

# Numbered, full-grid-width rows fill every visible row below the prompt:
# `ctl capture` itself defines the expectation, so cell geometry never
# enters the assert by hand.
COLS=$("$CTL" capture --json "$ID" | python3 -c 'import sys,json; print(json.load(sys.stdin)["cols"])')
PAT="$ROOT/pat"
python3 - "$PAT" "$COLS" <<'PY'
import sys
path, cols = sys.argv[1], int(sys.argv[2])
base = "EdgeSelectZq3"
with open(path, "w") as f:
    for i in range(80):
        row = ("%03d" % i + base * 11)[:cols]
        f.write(row + "\n")
PY
"$CTL" send "$ID" --text "cat $PAT"$'\n' >/dev/null
sleep 1
"$CTL" capture "$ID" > "$ROOT/cap.txt"
NROWS=$(wc -l < "$ROOT/cap.txt")
python3 - "$ROOT/cap.txt" "$COLS" <<'PY' || fail "screen is not fully covered by numbered rows"
import sys
lines = open(sys.argv[1]).read().splitlines()
cols = int(sys.argv[2])
base = "EdgeSelectZq3"
covered = [l for l in lines if l.startswith(tuple("%03d" % i for i in range(80))) and base in l and len(l) == cols]
sys.exit(0 if len(covered) >= len(lines) - 2 else 1)
PY
echo "grid ${COLS}x${NROWS}, numbered rows cover the screen"

step "drag into the RIGHT border strip selects through the last column"
drag_select $((X + 10)) $((Y + 80)) $((X + WIDTH - 2)) $((Y + 80))
CLIP=$(copy_selection | norm)
python3 - "$ROOT/cap.txt" "$CLIP" <<'PY' || { printf 'clip:\n%s\n' "$CLIP"; fail "right-edge selection lost text"; }
import sys
cap = [l.rstrip() for l in open(sys.argv[1]).read().splitlines()]
clip = sys.argv[2].splitlines()
# One line only. The press sat on column 1, so the first character of
# the numbered row is missing; the row is identified by its remaining
# text (3-digit numbering keeps [1:] unique) and must round-trip whole:
# exactly the eaten first char is allowed to differ.
one = clip[0] if len(clip) == 1 else ""
row = next((l for l in cap if l and l[1:] == one), "")
sys.exit(0 if one and row and len(one) == len(row) - 1 else 1)
PY
echo "right-edge copy OK ($(printf '%s' "$CLIP" | wc -c) bytes)"

step "drag into the BOTTOM-RIGHT corner strip selects through the last row"
drag_select $((X + 10)) $((Y + 80)) $((X + WIDTH - 2)) $((Y + HEIGHT - 2))
CLIP=$(copy_selection | norm)
python3 - "$ROOT/cap.txt" "$CLIP" <<'PY' || { diff <(printf '%s\n' "$CLIP") "$ROOT/cap.txt" | head -8; fail "bottom-edge selection lost text"; }
import sys
cap = [l.rstrip() for l in open(sys.argv[1]).read().splitlines()]
clip = sys.argv[2].splitlines()
# First line: pressed row with its first character eaten (press column 1);
# identified by remaining text, like above.
one = clip[0] if clip else ""
start = next((i for i, l in enumerate(cap) if l and l[1:] == one), None)
# Every row from the pressed row to the LAST grid row, cell-exact.
expected = [l[1:] if i == 0 else l for i, l in enumerate(cap[start:])] if start is not None else None
sys.exit(0 if expected is not None and clip == expected else 1)
PY
echo "bottom-edge copy OK ($(printf '%s' "$CLIP" | wc -l) rows)"

step 'PASS: border-strip selections copy the full edge text'
