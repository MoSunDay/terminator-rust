#!/usr/bin/env bash
# measure-cpu-remote.sh - run ONE cpu scenario from the S1-S13 perf matrix
# (agents/perf/index.md) against the REAL deploy box, sampling the live
# app's /proc/<pid>/stat through scripts/bin/cpu-sample.py.
#
# Design rules this script exists to honor:
#   * ONE ssh invocation per scenario: sandboxed/dev environments reap
#     processes between tool calls, so setup + sample + teardown must live
#     inside a single remote script (never a persistent ssh session).
#   * NEVER `pkill -f` a pattern that can occur in our own command line -
#     the app pid is resolved by scanning /proc/*/exe, killing nothing.
#   * The control socket is NOT assumed: the app's own environment
#     (/proc/<pid>/environ) is the source of TERMINATOR_SOCK /
#     XDG_RUNTIME_DIR / DISPLAY / XAUTHORITY for ctl + xdotool.
#
# Env contract (all overridable):
#   DEPLOY_HOST   ssh target, REQUIRED (e.g. m@192.168.31.196)
#   APP           process exe basename or full path, default terminator-rust
#   CTL           terminator-ctl command/path on the remote, default terminator-ctl
#   SCEN          comma list of scenario ids, default s1
#   CPU_SAMPLE    sampler path RELATIVE TO THE REMOTE REPO ROOT,
#                 default scripts/bin/cpu-sample.py
#   ROOT          remote repo root, default ~/terminator-rust
#   SAMPLE_SECONDS  sampler --seconds window, default 10
#   SAMPLER_ARGS    extra sampler args, e.g. --json (word-split; default empty)
#   WM_CLASS        xdotool --class for the app window, default terminator-rust
#   LOCAL=1         run the assembled remote script with the LOCAL bash
#                   instead of ssh (driver self-test only; the matrix runs
#                   the remote script on the deploy box)
#
# PID discovery: every /proc/<pid>/exe whose target matches APP (basename,
# or path when APP contains a slash) is a candidate. Several pids on the
# SAME resolved exe path -> newest /proc/<pid>/stat starttime wins (note
# printed). DIFFERENT exe paths -> refuse (ambiguous; set APP to the full
# path). 0 candidates -> refuse. `$CTL instances` is probed only as an
# informational cross-check (its PID column stays "-" for the primary
# socket, so it is NOT authoritative).
#
# Scenarios implemented remotely: s1 s2 s3 s4 s7 s8 s11 s13
#   (s5 s6 s9 s10 s12 are documented in agents/perf/index.md but need the
#   local e2e rig; s7 skips by design - pane-header double-click geometry).
# Remote exit codes: 0 ok / 2 setup failure (no pid, no pane, no ctl) /
# 3 skipped (missing xdotool/DISPLAY or a documented non-remote scenario).
# Local exit: 0 all ok (skips allowed), 1 any scenario failed.
#
# Usage: DEPLOY_HOST=m@box SCEN=s1,s2 SAMPLER_ARGS=--json \
#            scripts/deploy/measure-cpu-remote.sh
set -euo pipefail

DEFERRED="s5 s6 s9 s10 s12"        # matrix ids that only run on the local e2e rig

usage() { sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; }

# Arg handling runs BEFORE the required-env check so `-h` works anywhere.
case "${1:-}" in
    -h|--help) usage; exit 0 ;;
    "") ;;
    *) echo "measure-cpu-remote: unknown argument '$1' (env-driven; -h)" >&2; exit 2 ;;
esac

if [ "${LOCAL:-0}" = "1" ]; then
    echo "LOCAL=1: running the assembled script locally (driver self-test)" >&2
else
    : "${DEPLOY_HOST:?set DEPLOY_HOST to the ssh target (e.g. m@192.168.31.196), or LOCAL=1 to self-test}"
fi
APP=${APP:-terminator-rust}
CTL=${CTL:-terminator-ctl}
SCEN=${SCEN:-s1}
CPU_SAMPLE=${CPU_SAMPLE:-scripts/bin/cpu-sample.py}
ROOT=${ROOT:-~/terminator-rust}
SAMPLE_SECONDS=${SAMPLE_SECONDS:-10}
SAMPLER_ARGS=${SAMPLER_ARGS:-}
WM_CLASS=${WM_CLASS:-terminator-rust}
SSH_OPTS=${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=10}

TMP=$(mktemp -d /tmp/term-cpu-remote-XXXXXX)
trap 'rm -rf "$TMP"' EXIT

# --- remote prelude (runs on the deploy box, args after the extra --) ------
cat > "$TMP/prelude" <<'REMOTE'
set -u
root=$1; app=$2; ctl=$3; sampler=$4; secs=$5; scen=$6; extra=$7; wmclass=$8
case "$root" in "~") root=$HOME ;; "~/"*) root=$HOME/${root#\~/} ;; esac
cd "$root" 2>/dev/null || { echo "ERROR: remote repo root not found: $root" >&2; exit 2; }
[ -f "$sampler" ] || { echo "ERROR: sampler not found: $sampler (in $root)" >&2; exit 2; }

win_geom() {  # X Y WIDTH HEIGHT of $WID, empty on failure
    command -v xdotool >/dev/null 2>&1 || return 1
    xdotool getwindowgeometry --shell "$WID" 2>/dev/null \
        | sed -n 's/^X=//p; s/^Y=//p; s/^WIDTH=//p; s/^HEIGHT=//p' | tr '\n' ' '
}
MOVER=""; STOP=/tmp/term-cpu-stop-$$; ENVF=""
cleanup_remote() {
    rm -f "$STOP"
    [ -n "$ENVF" ] && rm -f "$ENVF"
    if [ -n "$MOVER" ]; then kill "$MOVER" 2>/dev/null; wait "$MOVER" 2>/dev/null; fi
    if command -v xdotool >/dev/null 2>&1; then xdotool mouseup 1 >/dev/null 2>&1; fi
}
trap cleanup_remote EXIT

# --- resolve the app pid: /proc/*/exe, never a socket guess ----------------
cands=()   # "pid starttime exe" for every exe matching $app
for exe in /proc/[0-9]*/exe; do
    [ -e "$exe" ] || continue
    tgt=$(readlink "$exe" 2>/dev/null) || continue
    tgt=${tgt% (deleted)}
    case "$app" in
        */*) case "$tgt" in "$app"|*/"$app") ;; *) continue ;; esac ;;
        *) [ "${tgt##*/}" = "$app" ] || continue ;;
    esac
    p=${exe#/proc/}; p=${p%/exe}
    # field 22 = starttime (after "pid (comm) ", field 22 is field 20)
    st=$(awk '{sub(/^[^)]*\) /,""); print $20}' "/proc/$p/stat" 2>/dev/null)
    cands+=("$p ${st:-0} $tgt")
done
if [ "${#cands[@]}" -eq 0 ]; then
    echo "ERROR: no process whose exe resolves to '$app' - is the app running? (APP=/full/path narrows it)" >&2
    exit 2
fi
n=${#cands[@]}
paths=$(printf '%s\n' "${cands[@]}" | awk '{print $3}' | sort -u)
npaths=$(printf '%s\n' "$paths" | wc -l)
if [ "$npaths" -gt 1 ]; then
    echo "ERROR: APP='$app' is ambiguous - $npaths different exe paths:" >&2
    printf '%s\n' "${cands[@]}" | sort -k2 -n >&2
    echo "set APP to the full path of the instance you want to measure" >&2
    exit 2
fi
PID=$(printf '%s\n' "${cands[@]}" | sort -k2 -n | tail -n1 | cut -d' ' -f1)
EXE_PATH=$(printf '%s\n' "$paths")
[ "$n" -gt 1 ] && echo "note: $n pids share $EXE_PATH; newest starttime wins (pid $PID)" >&2

ENVF=/tmp/term-cpu-env-$$
tr '\0' '\n' < "/proc/$PID/environ" 2>/dev/null > "$ENVF" || true
env_of() { sed -n "s/^$1=//p" "$ENVF" | head -n1; }
if [ ! -s "$ENVF" ]; then
    echo "WARN: cannot read /proc/$PID/environ - run this as the app's user (or root)" >&2
fi
sock=$(env_of TERMINATOR_SOCK); rt=$(env_of XDG_RUNTIME_DIR)
disp=$(env_of DISPLAY); xauth=$(env_of XAUTHORITY)
[ -n "$rt" ] && export XDG_RUNTIME_DIR="$rt"
[ -n "$sock" ] && export TERMINATOR_SOCK="$sock"
[ -n "$disp" ] && export DISPLAY="$disp"
[ -n "$xauth" ] && export XAUTHORITY="$xauth"
if grep -q '^TERMINATOR_FRAME_STATS=1$' "$ENVF"; then
    echo "frame-stats: ON (fps lines go to the app's stderr log)"
else
    echo "WARN: TERMINATOR_FRAME_STATS=1 is not set on pid $PID - no fps line (relaunch with it)" >&2
fi
floor=$(env_of TERMINATOR_FRAME_FLOOR_MS)
echo "frame floor: TERMINATOR_FRAME_FLOOR_MS=${floor:-unset (= app default 16ms)}"
echo "window: DISPLAY=${DISPLAY:-unset} sock=${TERMINATOR_SOCK:-default}"
echo "== $scen on $(hostname): pid=$PID exe=$EXE_PATH"
echo "   (secondaries: '| grep frame-stats' the app log for the fps line)"

if command -v "$ctl" >/dev/null 2>&1; then
    inst=$("$ctl" instances 2>/dev/null || true)
    ctl_pids=$(printf '%s\n' "$inst" | awk 'NR>1 {if ($4 ~ /^[0-9]+$/) print $4}' | sort -u | tr '\n' ' ')
    if [ -n "$ctl_pids" ]; then
        echo "   ctl instances pids: $ctl_pids (informational; primary socket shows '-')"
    fi
else
    echo "   note: ctl '$ctl' not on PATH - pane scenarios will fail, s1 works"
fi

WID=""
if [ -n "${DISPLAY:-}" ] && command -v xdotool >/dev/null 2>&1; then
    WID=$(xdotool search --class "$wmclass" 2>/dev/null | tail -n1 || true)
    [ -n "$WID" ] || echo "WARN: xdotool found no window with class '$wmclass'" >&2
fi

need_ctl() {
    command -v "$ctl" >/dev/null 2>&1 || { echo "ERROR: ctl '$ctl' not found (set CTL=...)" >&2; exit 2; }
}
first_pane() {
    "$ctl" list --json 2>/dev/null \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])'
}
need_pane() {
    PANE=$(first_pane) || true
    if [ -z "${PANE:-}" ]; then
        echo "ERROR: '$ctl list --json' gave no pane (socket=$sock?)" >&2
        exit 2
    fi
    echo "   pane=$PANE"
}
run_sampler() { python3 "$sampler" --pid "$PID" --seconds "$secs" $extra; }
stop_loop() { "$ctl" send "$PANE" --text $'\003' >/dev/null 2>&1 || true; }

# Background helpers used by s8/s11: bounded xdotool loops that stop as
# soon as the sampler is done (STOP file) - cleanup_remote also kills them.
# Every move is wrapped in `timeout 1`: a --sync relative move can wait
# forever when it clamps at a screen edge, and one stuck move must not
# wedge the whole scenario. NEGATIVE offsets are avoided entirely (the
# parser treats them as options): motion is a forward relative step plus
# an absolute reset every $4 steps ($2/$3 = reset x/y, may be empty).
spin_mover() {
    ( n=0
      while [ ! -e "$STOP" ]; do
          timeout 1 xdotool mousemove_relative --sync "$1" 0 >/dev/null 2>&1 || break
          n=$((n + 1))
          if [ -n "$2" ] && [ $((n % $4)) -eq 0 ]; then
              timeout 1 xdotool mousemove --sync "$2" "$3" >/dev/null 2>&1 || break
          fi
      done ) &
    MOVER=$!
}
end_mover() {
    rm -f "$STOP"
    if [ -n "$MOVER" ]; then kill "$MOVER" 2>/dev/null; wait "$MOVER" 2>/dev/null; MOVER=""; fi
}
REMOTE

# --- scenario bodies (remote text; appended to the prelude) ----------------

body_s1() {
    cat <<'REMOTE'

echo "setup: none (idle window)"
run_sampler
REMOTE
}

body_s2() {
    cat <<'REMOTE'

need_ctl; need_pane
echo "setup: 60Hz spinner into pane $PANE (single-quoted payload, CR line)"
"$ctl" send "$PANE" --text $'while :; do printf "spin %s\\r" "$(date +%s%N)"; sleep 0.016; done\n' >/dev/null
sleep 1
run_sampler
echo "teardown: ^C to the pane"
stop_loop
REMOTE
}

body_s3() {
    cat <<'REMOTE'

need_ctl; need_pane
echo "setup: maximize the window (WM-DEPENDENT, best-effort)"
if [ -n "$WID" ]; then
    xdotool windowsize --sync "$WID" 100% 100% && echo "   resized $WID to 100% 100%"
    xdotool windowactivate "$WID" 2>/dev/null || true
else
    echo "   WARN: no xdotool/DISPLAY/WID - window left as-is, measurement still runs" >&2
fi
sleep 1
echo "setup: 60Hz spinner into pane $PANE"
"$ctl" send "$PANE" --text $'while :; do printf "spin %s\\r" "$(date +%s%N)"; sleep 0.016; done\n' >/dev/null
sleep 1
run_sampler
echo "teardown: ^C to the pane"
stop_loop
REMOTE
}

body_s4() {
    cat <<'REMOTE'

need_ctl; need_pane
echo "setup: TUI-style full-screen redraw (cursor home + 40 lines, 60Hz)"
"$ctl" send "$PANE" --text $'while :; do printf \'\\033[H\'; seq 1 40 | sed \'s/^/line /\'; sleep 0.016; done\n' >/dev/null
sleep 1
run_sampler
echo "teardown: ^C to the pane"
stop_loop
REMOTE
}

body_s7() {
    cat <<'REMOTE'

echo "SKIP: s7 (rename editor open, no composition) is not driven remotely:"
echo "  the pane-header double-click geometry depends on the live layout;"
echo "  covered by the local IME e2e instead (agents/input-method/index.md)."
exit 3
REMOTE
}

body_s8() {
    cat <<'REMOTE'

echo "setup: continuous mouse motion over the app (xdotool, best-effort)"
if [ -z "$WID" ]; then echo "SKIP: no xdotool/DISPLAY/WID" >&2; exit 3; fi
xdotool windowactivate "$WID" 2>/dev/null || true
# park the pointer at the window center, then step 20px right at a time
# with an absolute reset every 15 steps: stays inside the window, no
# screen-edge clamp and no negative (option-looking) offsets
geo=$(win_geom) || geo=""
if [ -n "$geo" ]; then
    set -- $geo
    spin_mover 20 $(( $1 + $3 / 2 )) $(( $2 + $4 / 2 )) 15
else
    echo "   WARN: no window geometry - moving from wherever the pointer is" >&2
    spin_mover 20 "" "" 5
fi
sleep 0.3
run_sampler
rc=$?
end_mover
exit "$rc"
REMOTE
}

body_s11() {
    cat <<'REMOTE'

echo "setup: window edge-resize gesture (drag the right edge; WM best-effort)"
if [ -z "$WID" ]; then echo "SKIP: no xdotool/DISPLAY/WID" >&2; exit 3; fi
geo=$(win_geom) || geo=""
if [ -z "$geo" ]; then echo "SKIP: xdotool getwindowgeometry failed" >&2; exit 3; fi
set -- $geo
edge=$(( $1 + $3 - 2 )); midy=$(( $2 + $4 / 2 ))
xdotool windowactivate "$WID" 2>/dev/null || true
echo "   press right edge at x=$edge y=$midy (window $1,$2 $3x$4)"
timeout 1 xdotool mousemove --sync "$edge" "$midy"
sleep 0.3
xdotool mousedown 1
# while the button is held the grabbed edge follows the pointer: 20px
# right (grow) then an absolute reset back to $edge (shrink), repeating
spin_mover 20 "$edge" "$midy" 1
sleep 0.3
run_sampler
rc=$?
end_mover
xdotool mouseup 1
exit "$rc"
REMOTE
}

body_s13() {
    cat <<'REMOTE'

need_ctl; need_pane
BIG=/tmp/term-cpu-flood-$$
seq 1 40000 | sed 's/^/line /' > "$BIG"
echo "setup: cat 40000 lines ($(wc -c < "$BIG") bytes) - record-only,"
echo "  output-bandwidth-bound, outside the <1% contract"
"$ctl" send "$PANE" --text $'cat '"$BIG"$'\n' >/dev/null
sleep 0.2
run_sampler
rm -f "$BIG"
REMOTE
}

# --- local driver: one ssh per scenario ------------------------------------

run_remote() {  # $1 = script file; remaining args = remote argv
    local body=$1
    shift
    if [ "${LOCAL:-0}" = "1" ]; then
        bash -s -- "$@" < "$body"
    else
        ssh $SSH_OPTS "$DEPLOY_HOST" bash -s -- "$@" < "$body"
    fi
}

run_scenario() {
    local s=$1 rc=0
    if ! declare -F "body_$s" >/dev/null 2>&1; then
        if printf ' %s ' "$DEFERRED" | grep -q " $s "; then
            echo "== SKIP $s (documented local-only matrix scenario; agents/perf/index.md)"
            return 0
        fi
        echo "FAIL: unknown scenario '$s' (implemented: s1 s2 s3 s4 s7 s8 s11 s13)" >&2
        return 1
    fi
    { cat "$TMP/prelude"; "body_$s"; } > "$TMP/body.sh"
    if [ "${LOCAL:-0}" = "1" ]; then
        echo "== $s: LOCAL self-test (no ssh)"
    else
        echo "== $s: one ssh invocation -> $DEPLOY_HOST"
    fi
    if run_remote "$TMP/body.sh" \
        "$ROOT" "$APP" "$CTL" "$CPU_SAMPLE" "$SAMPLE_SECONDS" "$s" \
        "$SAMPLER_ARGS" "$WM_CLASS"; then
        rc=0
    else
        rc=$?
    fi
    case "$rc" in
        0) echo "== $s done" ;;
        3) echo "== $s SKIPPED (see the message above)" ;;
        *) echo "== $s FAILED (remote exit $rc)" >&2; return 1 ;;
    esac
}

overall=0
for s in $(printf '%s' "$SCEN" | tr ',' ' '); do
    [ -n "$s" ] || continue
    run_scenario "$s" || overall=1
done
if [ "$overall" -ne 0 ]; then
    echo "FAIL: at least one scenario failed" >&2
    exit 1
fi
echo "PASS: every requested scenario ran (skips allowed; see lines above)"
