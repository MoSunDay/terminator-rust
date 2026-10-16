#!/usr/bin/env bash
# e2e-idle-cpu.sh - reactive render loop + idle CPU budget gate.
#
# The render loop is REACTIVE (render/cadence.rs): no unconditional tick
# exists, PTY/IPC output wakes request repaints on demand, and an idle
# terminal must not repaint at all. This script drives a sandboxed app
# under Xvfb and checks that contract with two instruments:
#   - scripts/bin/cpu-sample.py --pid <app>: utime+stime from
#     /proc/<pid>/stat, peak 1-second bucket ("max1s", % of ONE core);
#   - the app's own frame-stats stderr lines (TERMINATOR_FRAME_STATS=1,
#     render/stats.rs): one line per second that had frames, so "no new
#     frame-stats line" is exactly fps == 0.
#
# Phases (see agents/perf/index.md for the S1..S13 matrix):
#   S1  idle      ASSERTED: max1s < 1% of one core AND fps == 0.
#       The perf contract. Anything that wakes the loop (a 16/50ms tick,
#       an unpinned cursor blink, a poll from a gesture) shows up here.
#   S1b idle with the pane cursor pinned (DECTCEM off -> no blink edge).
#       Recorded, not asserted: it separates "the loop still schedules
#       frames" from "the host burns CPU with zero frames" (software
#       Vulkan/llvmpipe Xvfb hosts have a nonzero floor - see below).
#   WAKE roundtrip through the idle app: `ctl send` writes into a pane and
#       the echoed marker must show up in `ctl capture` in <= 2s. With no
#       tick left this only works if the pty-reader and IPC wakes request
#       repaints.
#   S7  pane rename editor: opened by a pane-header title double-click and
#       held open while sampling (recorded) + a runaway guard. The editor
#       is the IME latch path (focus_rename_editor): a regression there
#       holds the keyboard and re-renders every frame. Also asserts the
#       typed name never leaks into the pane and is committed on Enter.
#   S2  60Hz spinner: `printf ... \\r` every 16ms. ASSERTED fps <= 65:
#       output pacing (TERMINATOR_FRAME_FLOOR_MS, default 16ms) must keep
#       output-driven frames at/below the output rate, never faster.
#   S13 busy: `cat` a 40k-line file. ASSERTED fps <= 65, CPU recorded.
#
# `frames=N` in every report below counts frame-stats LINES in the
# window, i.e. the seconds that had at least one frame: fps == 0 is
# exactly "no new line".
#
# The sandbox pins TERMINATOR_NO_MOTION=1 (e2e-lib.sh), whose contract is
# to hold the hover fades AND the cursor blink at their end states - the
# S1 fps == 0 assert IS that pin (render/tokens.rs cursor_alpha returns
# 1.0 under the pin, screen.rs gates blink_text/blink_on on it). S1b
# pins the CURSOR instead (DECTCEM off) and shows what is left.
#
# GPU BACKEND (IDLE_GPU_BACKEND, default `gl`): S1 is a CPU gate on OUR
# render work, and the frame loop is asleep (the app's own threads
# measure 0 jiffies over 12s), but host GPU drivers are not: on this
# sandbox the NVIDIA Vulkan stack keeps a present/poll thread at
# ~0.3-0.4% of a core with ZERO frames rendered, which alone puts 1-2
# jiffy seconds (1-2%) into max1s - the 10ms tick quantum makes that a
# hard fail for a process that is otherwise fully blocked. Pinning the
# software backend (llvmpipe via WGPU_BACKEND=gl) removes the driver
# floor so the gate measures the app loop; `IDLE_GPU_BACKEND=native`
# measures the real driver too (a real desktop's gate). The fps
# ceilings assume the default 16ms pacing floor: exporting
# TERMINATOR_FRAME_FLOOR_MS=0 (unthrottled upper bound) is a measurement
# mode, not a gate mode, and will trip the S2/S13 ceilings on purpose.
#
# Usage: scripts/bin/e2e-idle-cpu.sh   (repo root; Xvfb + xdotool +
# python3 needed). Env:
#   E2E_KEEP=1          keep the scratch dir (app.log has every
#                       frame-stats line - the first thing to read)
#   APP_BIN/CTL_BIN     run binaries outside target/debug (skips the
#                       cargo build; scripts/deploy/verify-release.sh
#                       points them at the installed release bundle)
#   IDLE_SAMPLE_S=8     S1 window seconds
#   IDLE_SETTLE_S=6     quiet time between the setup traffic (ctl
#                       list/capture) and the S1 sample: the app serves
#                       those over IPC, and the first GL frame's
#                       rasterizer jiffies land seconds after launch on
#                       a slow software backend - both would otherwise
#                       spend a 10ms jiffy (= 1.0% max1s) inside the
#                       idle window
#   IDLE_MAX1S_PCT=1.0  S1 CPU ceiling (% of one core)
#   IDLE_MAX_FPS=0      S1 frame-seconds ceiling
#   IDLE_GPU_BACKEND=gl software backend for the S1 sample (`native`
#                       keeps the host driver; see "GPU BACKEND")
#   PIN_SAMPLE_S=5      S1b window seconds
#   S7_SAMPLE_S=5       S7 window seconds
#   S7_MAX1S_PCT=50     S7 runaway guard (% of one core)
#   MAX_FPS_SPIN=65     S2 frame-rate ceiling
#   MAX_FPS_BUSY=65     S13 frame-rate ceiling
set -euo pipefail
source "$(dirname "$0")/e2e-lib.sh"
cd "$(dirname "$0")/../.."

APP=${APP_BIN:-target/debug/terminator-rust}
CTL=${CTL_BIN:-target/debug/terminator-ctl}
IDLE_SAMPLE_S=${IDLE_SAMPLE_S:-8}
IDLE_SETTLE_S=${IDLE_SETTLE_S:-6}
IDLE_MAX1S_PCT=${IDLE_MAX1S_PCT:-1.0}
IDLE_MAX_FPS=${IDLE_MAX_FPS:-0}
IDLE_GPU_BACKEND=${IDLE_GPU_BACKEND:-gl}
PIN_SAMPLE_S=${PIN_SAMPLE_S:-5}
S7_SAMPLE_S=${S7_SAMPLE_S:-5}
S7_MAX1S_PCT=${S7_MAX1S_PCT:-50}
MAX_FPS_SPIN=${MAX_FPS_SPIN:-65}
MAX_FPS_BUSY=${MAX_FPS_BUSY:-65}
SPIN_STEP_S=0.016
SPIN_N=400

ROOT=$(mktemp -d /tmp/term-e2e-cpu-XXXXXX)
# shellcheck disable=SC2034  # read by fail() in e2e-lib.sh (tails it)
E2E_FAIL_LOG="$ROOT/app.log"
DISPLAY_N=""
XVFB_PID=""
APP_PID=""
WID=""
X=0; Y=0; WIDTH=0   # geo() also sets HEIGHT; only these three are read
cleanup() { e2e_cleanup "$APP_PID" "$XVFB_PID"; }
trap cleanup EXIT

idle_frames=0; idle_max1s=0; idle_avg=0
pin_frames=0; pin_max1s=0; pin_avg=0
wake_ms=0
s7_max1s=0; s7_frames=0
spin_maxfps=0; spin_max1s=0
busy_maxfps=0; busy_max1s=0; busy_pass=0us

if [ -z "${APP_BIN:-}" ] && [ -z "${CTL_BIN:-}" ]; then
    # Build first: the sandboxed HOME below would hide rustup/toolchains.
    step "cargo build -p app -p ctl --bins"
    cargo build -p app -p ctl --bins >/dev/null
fi
[ -x "$APP" ] || fail "app binary is not executable: $APP (set APP_BIN)"
[ -x "$CTL" ] || fail "ctl binary is not executable: $CTL (set CTL_BIN)"

e2e_sandbox /bin/bash 0
e2e_start_xvfb 1200x800x24
# The instrumented app itself reports frames (stderr, <=1 line/s).
export TERMINATOR_FRAME_STATS=1
# S7 must be composition-free: a stray XIM server (a shared box's ibus
# --xim) would eat the ASCII rename keys into a preedit instead.
unset XMODIFIERS || true
step "app=$APP ctl=$CTL display=$DISPLAY_N floor=${TERMINATOR_FRAME_FLOOR_MS:-16(default)}ms backend=$IDLE_GPU_BACKEND"

GPU_ENV=()
if [ "$IDLE_GPU_BACKEND" != "native" ] && [ -n "$IDLE_GPU_BACKEND" ]; then
    GPU_ENV=(WGPU_BACKEND="$IDLE_GPU_BACKEND")
fi
env DISPLAY="$DISPLAY_N" RUST_LOG=info ${GPU_ENV[@]+"${GPU_ENV[@]}"} setsid "$APP" >"$ROOT/app.log" 2>&1 &
APP_PID=$!
for _ in $(seq 1 60); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || fail "ipc socket never appeared at $SOCK (app log: $(tail -3 "$ROOT/app.log" | tr '\n' ' '))"
# setsid execs into the app, so $! is the app pid; pgrep is a fallback.
if [ ! -r "/proc/$APP_PID/stat" ]; then
    ALT=$(pgrep -x terminator-rust | tail -1 || true)
    [ -n "$ALT" ] || fail "no /proc entry for the app pid $APP_PID"
    echo "warn: falling back to pgrep pid $ALT"
    APP_PID=$ALT
fi
WID=$(wait_win '^terminator-rust$') || fail "app window never appeared"
# Let the prompt + startup frames settle before the first sample.
sleep 3

# --- instruments ----------------------------------------------------------

# frame_report <log-line-mark>: "<count> <max_fps>" over the frame-stats
# lines that appeared AFTER log line <mark> (stats.rs prints exactly one
# line per second that had frames, so an empty window is fps == 0).
frame_report() {
    tail -n "+$(( $1 + 1 ))" "$ROOT/app.log" | python3 -c '
import re, sys
fps = [float(m.group(1)) for m in (re.search(r"fps=([0-9.]+)", l) for l in sys.stdin) if m]
print(len(fps), max(fps) if fps else 0.0)
'
}
# max_pass_us <log-line-mark>: longest single pass in the window (us),
# 0 when none - a flood that does not finish inside one bucket shows up
# here as one enormous pass (the pass is not chunked).
max_pass_us() {
    tail -n "+$(( $1 + 1 ))" "$ROOT/app.log" | python3 -c '
import re, sys
v = [int(m.group(1)) for m in (re.search(r"pass_max_us=([0-9]+)", l) for l in sys.stdin) if m]
print(max(v) if v else 0)
'
}
lines_now() { wc -l < "$ROOT/app.log"; }
last_stats() { grep -m1 'frame-stats' "$ROOT/app.log" | tail -1 || true; }
top_threads() {
    python3 -c 'import json,sys
d = json.loads(sys.argv[1])
print(", ".join("%s %.2f%%" % (t["comm"], t["pct"]) for t in d["threads"][:3]))' "$1"
}
json_num() { python3 -c 'import json,sys; print(json.loads(sys.argv[1])[sys.argv[2]])' "$1" "$2"; }
float_lt() { python3 -c 'import sys; sys.exit(0 if float(sys.argv[1]) < float(sys.argv[2]) else 1)' "$1" "$2"; }
float_le() { python3 -c 'import sys; sys.exit(0 if float(sys.argv[1]) <= float(sys.argv[2]) else 1)' "$1" "$2"; }
# cpu_sample <seconds>: cpu-sample.py JSON for the app process.
cpu_sample() { python3 scripts/bin/cpu-sample.py --pid "$APP_PID" --seconds "$1" --json; }

PANE=$("$CTL" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])') \
    || fail "ctl list --json failed: no pane to drive"
[ -n "$PANE" ] || fail "ctl list --json returned no pane"
cap() { "$CTL" capture "$PANE" 2>/dev/null || true; }

[ "$(frame_report 0 | cut -d' ' -f1)" -gt 0 ] \
    || fail "no frame-stats line at all: TERMINATOR_FRAME_STATS=1 did not reach the app (stderr instrumentation missing), the fps asserts would pass vacuously"

# --- S1: idle (asserted) --------------------------------------------------

step "S1 idle: settle ${IDLE_SETTLE_S}s, cpu-sample ${IDLE_SAMPLE_S}s, assert max1s < ${IDLE_MAX1S_PCT}% and fps == ${IDLE_MAX_FPS}"
sleep "$IDLE_SETTLE_S"
mark=$(lines_now)
J=$(cpu_sample "$IDLE_SAMPLE_S")
read -r idle_frames idle_maxfps <<<"$(frame_report "$mark")"
idle_max1s=$(json_num "$J" max1s_pct)
idle_avg=$(json_num "$J" avg_pct)

# Numbers first: they are the evidence whether or not an assert fires.
echo "idle: avg=${idle_avg}% max1s=${idle_max1s}% frames=${idle_frames} maxfps=${idle_maxfps} in ${IDLE_SAMPLE_S}s"
float_le "$idle_frames" "$IDLE_MAX_FPS" \
    || fail "S1 idle: ${idle_frames} frame-stats line(s) in ${IDLE_SAMPLE_S}s (want fps == ${IDLE_MAX_FPS}, last: $(last_stats)). Frames come from cadence deadlines: while the user-focused pane's cursor blinks the loop schedules every blink edge (render/tokens.rs cursor_alpha ignores TERMINATOR_NO_MOTION, screen.rs cursor_blink). Fix that or set IDLE_MAX_FPS to the observed steady rate."
float_lt "$idle_max1s" "$IDLE_MAX1S_PCT" \
    || fail "S1 idle: max1s=${idle_max1s}% >= ${IDLE_MAX1S_PCT}% (avg=${idle_avg}%, frames=${idle_frames}, backend=$IDLE_GPU_BACKEND, top: $(top_threads "$J")). Probe the app's own threads first (cpu-sample --top-threads): the loop must show 0 jiffies. TWO causes look alike at this resolution: one 10ms jiffy in the window IS exactly 1.0% (a cold release bundle pays page-in + font parsing; re-run once before investigating, or raise IDLE_SETTLE_S), and a nonzero floor with 0 frames is the host (GPU driver) - re-run with the default IDLE_GPU_BACKEND=gl, or raise IDLE_MAX1S_PCT on a documented-floors host."

# --- S1b: floor probe with the cursor pinned (recorded) -------------------

step "S1b floor probe: cursor pinned (no blink edge), cpu-sample ${PIN_SAMPLE_S}s"
# DECTCEM off: the pane stops reporting a visible cursor, so no blink
# deadline is armed - this is the local stand-in for the NO_MOTION pin
# that S1 wants from the app.
"$CTL" send "$PANE" --text "printf '\\033[?25l'"$'\n' >/dev/null
sleep 1.5
mark=$(lines_now)
J=$(cpu_sample "$PIN_SAMPLE_S")
read -r pin_frames pin_maxfps <<<"$(frame_report "$mark")"
pin_max1s=$(json_num "$J" max1s_pct)
pin_avg=$(json_num "$J" avg_pct)
echo "pinned: avg=${pin_avg}% max1s=${pin_max1s}% frames=${pin_frames} maxfps=${pin_maxfps} (recorded, not asserted)"
"$CTL" send "$PANE" --text "printf '\\033[?25h'"$'\n' >/dev/null

# --- wake roundtrip through the idle app ----------------------------------

step "wake roundtrip (pty + ipc) through the idle app"
MARKER="WAKE_OK_$RANDOM"
"$CTL" send "$PANE" --text "echo $MARKER"$'\n' >/dev/null
seen=""
START=$(date +%s%N)
for _ in $(seq 1 40); do
    if cap | grep -qF -- "$MARKER"; then
        seen=yes
        break
    fi
    sleep 0.05
done
END=$(date +%s%N)
[ -n "$seen" ] || { cap; fail "marker never appeared: pty/ipc wake is broken"; }
wake_ms=$(( (END - START) / 1000000 ))
echo "wake roundtrip: marker visible after ~${wake_ms}ms (needs no repaint tick)"
[ "$wake_ms" -le 2000 ] || fail "wake took ${wake_ms}ms - output only arrived via a fallback path"

# --- S7: pane rename editor (recorded + runaway guard) --------------------

step "S7 pane rename editor: header double-click, hold, sample, commit"
# The pane-header title is centered in the header band; at the default
# font the band's centre is y+53 (e2e-ime.sh M5 uses the same probe).
xdotool windowfocus "$WID" 2>/dev/null || true
geo
xdotool mousemove $((X + WIDTH / 2)) $((Y + 53)); sleep 0.4
xdotool click --repeat 2 --delay 120 1
sleep 1.0
S7_NAME="s7probe$RANDOM"
# The editor opens pre-seeded with the current title and puts the caret at
# its end: select-all first so the committed name is exactly S7_NAME.
xdotool key ctrl+a
xdotool type --delay 60 "$S7_NAME"
sleep 0.3
cap | grep -qF -- "$S7_NAME" \
    && fail "S7: typed keys leaked into the pane - the rename editor never took the keyboard focus"
# Sample the HELD-OPEN editor (the P1 latch baseline), not the typing burst.
sleep 1.0
mark=$(lines_now)
J=$(cpu_sample "$S7_SAMPLE_S")
read -r s7_frames s7_maxfps <<<"$(frame_report "$mark")"
s7_max1s=$(json_num "$J" max1s_pct)
s7_avg=$(json_num "$J" avg_pct)
echo "s7: avg=${s7_avg}% max1s=${s7_max1s}% frames=${s7_frames} maxfps=${s7_maxfps} (recorded)"
float_lt "$s7_max1s" "$S7_MAX1S_PCT" \
    || fail "S7: max1s=${s7_max1s}% >= ${S7_MAX1S_PCT}% - the rename editor (IME latch path) stormed the loop (top: $(top_threads "$J"))"
xdotool key Return
sleep 1.0
NAME=$("$CTL" list --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0].get("name") or "")')
[ "$NAME" = "$S7_NAME" ] \
    || fail "S7: pane title after Enter is '$NAME', expected '$S7_NAME' (editor never opened, or the commit was rejected)"
cap | grep -qF -- "$S7_NAME" \
    && fail "S7: the probe name reached the pane shell - keys leaked out of the editor"
echo "s7: rename editor held '${S7_NAME}', committed on Enter, no key leak"

# --- S2: 60Hz spinner -----------------------------------------------------

step "S2 60Hz spinner: output-driven frames must stay <= ${MAX_FPS_SPIN}fps"
# Ceiling derivation: the 16ms pacing floor allows 62.5 output-driven
# frames/s, the blinking cursor adds ~2 edges/s -> <= 64.5. A ceiling of
# 65 therefore means "the floor held"; unthrottled output (FLOOR=0) or a
# lost pacing gate runs far past it.
# Double backslash: the pane's printf must see \\r as an escape (a single
# \033-style literal byte would be mangled by readline - see agents.md).
SPIN_CMD="i=0; while [ \$i -lt $SPIN_N ]; do printf \"spin %s\\\\r\" \"\$(date +%s%N)\"; sleep $SPIN_STEP_S; i=\$((i+1)); done"
"$CTL" send "$PANE" --text "$SPIN_CMD"$'\n' >/dev/null
sleep 0.5
mark=$(lines_now)
J=$(cpu_sample "$(python3 -c "print($SPIN_N * $SPIN_STEP_S - 1.0)")")
read -r spin_frames spin_maxfps <<<"$(frame_report "$mark")"
spin_max1s=$(json_num "$J" max1s_pct); spin_avg=$(json_num "$J" avg_pct)
echo "spinner: avg=${spin_avg}% max1s=${spin_max1s}% frames=${spin_frames} maxfps=${spin_maxfps}"
[ "$spin_frames" -gt 0 ] \
    || fail "S2: no frames at all while the spinner ran (frame-stats off, or the send never executed)"
float_le "$spin_maxfps" "$MAX_FPS_SPIN" \
    || fail "S2: spinner ran at ${spin_maxfps}fps > ${MAX_FPS_SPIN} - output pacing (TERMINATOR_FRAME_FLOOR_MS) is not holding"
cap | grep -qE 'spin [0-9]' || fail "S2: no spinner output in the pane (the payload never ran)"
# Let the bounded loop end AND its trailing frame-stats bucket close, so
# none of its frames leak into the S13 window below.
sleep 3

# --- S13: busy output (fps asserted, CPU recorded) ------------------------

step "S13 busy: cat a 40k-line file, fps <= ${MAX_FPS_BUSY}"
python3 - "$ROOT/big.txt" <<'PY'
import sys
with open(sys.argv[1], "w") as f:
    for i in range(40000):
        f.write(f"line {i} the quick brown fox jumps over the lazy dog\n")
PY
mark=$(lines_now)
"$CTL" send "$PANE" --text "cat $ROOT/big.txt"$'\n' >/dev/null
J=$(cpu_sample 5)
for _ in $(seq 1 40); do cap | grep -q 'line 39999' && break; sleep 0.25; done
read -r busy_frames busy_maxfps <<<"$(frame_report "$mark")"
busy_max1s=$(json_num "$J" max1s_pct); busy_avg=$(json_num "$J" avg_pct)
# A slow (debug) build chews the flood inside ONE pass, so frames/maxfps
# here can be ~0 - the ceiling below is what is asserted, the pass time
# is the number to watch (recorded: S13 is informational, the perf doc
# keeps the flood out of the <1% contract).
busy_pass=$(awk -v u="$(max_pass_us "$mark")" 'BEGIN { printf "%.2fs", u / 1e6 }')
echo "busy: avg=${busy_avg}% max1s=${busy_max1s}% frames=${busy_frames} maxfps=${busy_maxfps} longest_pass=${busy_pass}"
cap | grep -q 'line 39999' || fail "S13: the cat output never finished in the pane"
float_le "$busy_maxfps" "$MAX_FPS_BUSY" \
    || fail "S13: busy output ran at ${busy_maxfps}fps > ${MAX_FPS_BUSY} - pacing broken or FLOOR=0 in the environment"

step "PASS: idle max1s=${idle_max1s}% frames=${idle_frames}; pinned max1s=${pin_max1s}% frames=${pin_frames}; wake=${wake_ms}ms; s7 max1s=${s7_max1s}% frames=${s7_frames}; spinner fps=${spin_maxfps} max1s=${spin_max1s}%; busy fps=${busy_maxfps} max1s=${busy_max1s}% pass=${busy_pass}"
