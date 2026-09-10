#!/usr/bin/env bash
# Headless e2e for the mouse + keyboard input paths of terminator-rust,
# driven through REAL X events (Xvfb + xdotool) and verified over the live
# control socket with terminator-ctl:
#   T1: bare Ctrl+C still reaches the child pty as ^C even though egui-winit
#       folds it into Event::Copy (the app's ctrl-without-shift gate).
#   T2: a press -> drag -> release that starts in the LEFT pane and ends
#       over the RIGHT pane still delivers the SGR release bytes to the
#       LEFT pane's child (per-button pointer grabs; P0 = the release).
#   T3: Shift+PageUp pages the focused pane's local viewport back and
#       Shift+End returns to live (capture reflects the viewport).
#   T4: Ctrl+C must actually INTERRUPT a foreground job (SIGINT, not just
#       the ^C caret echo). Guards the pty.rs SIG_DFL reset (the
#       historical inherited-SIG_IGN bug).
#
# Usage: scripts/bin/e2e-mouse-key.sh   (from the repo root; needs Xvfb +
# xdotool). Set E2E_KEEP=1 to keep the scratch dir for debugging.
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-mk-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
DISPLAY_N=":$$"          # unique per run; no clash with parallel scripts
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

fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo "== $*"; }

# wait_grep <pane> <bre-pattern> [tries]: capture until the pattern shows.
wait_grep() {
    local pane=$1 pat=$2 tries=${3:-40}
    for _ in $(seq 1 "$tries"); do
        "$CTL" capture "$pane" 2>/dev/null | grep -q "$pat" && return 0
        sleep 0.25
    done
    return 1
}

# pane_pid <pane>: pid of the pane's direct child from `list --json`.
# <pane> may be a manual title ("agent1") or a numeric pane id. Always
# returns 0 (empty stdout = not found) so it is safe under set -e in $( ).
pane_pid() {
    local sel=$1
    if [ -n "$sel" ] && [ "$sel" = "$(echo "$sel" | tr -cd 0-9)" ]; then
        "$CTL" list --json 2>/dev/null \
            | grep -A8 "\"id\": $sel," | grep '"pid"' | head -1 | grep -o '[0-9]\+' || true
    else
        # PaneInfo field order: id, name, ..., pid - pid sits AFTER name.
        "$CTL" list --json 2>/dev/null \
            | grep -A8 "\"name\": \"$sel\"" | grep '"pid"' | head -1 | grep -o '[0-9]\+' || true
    fi
}

# kid_of <pid>: pids of the direct children of <pid> ("" when none).
kid_of() { ps -o pid= --ppid "$1" 2>/dev/null | tr -d ' ' || true; }

# Build first: the sandboxed HOME below would hide rustup/toolchains.
cargo build -p app -p ctl --bins >/dev/null

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
export SHELL=/bin/bash   # pane children must be interactive bash (^C echo, printf \e)
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
# No compositor in Xvfb: pin full opacity for deterministic rendering.
export TERMINATOR_OPAQUE=1
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

cat > "$XDG_CONFIG_HOME/terminator-rust/state.json" <<'JSON'
{
  "theme": "catppuccin-mocha",
  "tabs": [
    { "title": "shell", "focused": 1,
      "root": { "Pane": { "id": 1, "meta": {
        "kind": "Local", "manual_title": "agent1",
        "bg": null, "transparency": 0.0, "degraded": false } } } }
  ]
}
JSON

# --- Xvfb + app ----------------------------------------------------------
step "launch Xvfb $DISPLAY_N + app"
Xvfb "$DISPLAY_N" -screen 0 1200x800x24 & XVFB_PID=$!
sleep 0.7
export DISPLAY="$DISPLAY_N"   # for xdotool (the app gets it via env below)
# NOTE: this `&` launch leaves SIGINT/SIGQUIT = SIG_IGN in the app (bash
# does that to async children when job control is off) - exactly what T4
# is designed to catch if the pane child does not reset dispositions.
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "control socket never appeared"; }
# capture/send is full remote control: the socket must be owner-only.
[ "$(stat -c %a "$SOCK")" = "600" ] || fail "socket perms $(stat -c %a "$SOCK") != 600"
sleep 2   # let the pane's shell boot

# --- window geometry (absolute screen coords; do NOT assume 0,0) ---------
step "locate window"
WID=""
for _ in $(seq 1 20); do
    WID=$(xdotool search --name terminator-rust 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.25
done
[ -n "$WID" ] || { tail "$ROOT/app.log"; fail "app window not found"; }
xdotool windowfocus "$WID"   # no WM: key events need explicit X focus
eval "$(xdotool getwindowgeometry --shell "$WID")"   # -> X Y WIDTH HEIGHT
echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"

# Content-safe click line: below tab bar + pane headers, above the bottom.
LX=$((X + WIDTH / 4))            # center of the left (agent1) pane
RX=$((X + 3 * WIDTH / 4))        # center of the right (post-split) pane
MY=$((Y + HEIGHT * 57 / 100))

# --- T1: bare Ctrl+C bytes reach the child (^C caret echo) ---------------
step "T1: bare Ctrl+C reaches the child (^C caret echo)"
xdotool mousemove "$LX" "$MY" click 1   # focus pane 1 (belt and braces)
sleep 0.4
"$CTL" send agent1 --text "sleep 45"$'\n' >/dev/null
sleep 0.6
xdotool key ctrl+c
wait_grep agent1 '\^C' 40 \
    || { "$CTL" capture agent1; tail "$ROOT/app.log"; fail "ctrl+c echo missing (egui Copy-fold gate regressed?)"; }
"$CTL" capture agent1 | grep -q 'sleep 45' \
    || { "$CTL" capture agent1; fail "ctrl+c sanity: typed 'sleep 45' not echoed"; }

# If Ctrl+C did not actually interrupt the job (see T4), the pane's bash is
# still in waitpid and would swallow everything we send next. Recover the
# prompt by killing the leftover sleep directly, so T2/T3 can proceed; T4
# re-tests the interrupt behavior properly and fails loudly if it is broken.
P1=$(pane_pid agent1)
[ -n "$P1" ] || fail "could not read pane 'agent1' pid from list --json"
LEFTOVER=$(kid_of "$P1" | head -1)
if [ -n "$LEFTOVER" ]; then
    echo "note: Ctrl+C did not interrupt 'sleep 45' (pane child $(grep SigIgn /proc/$LEFTOVER/status 2>/dev/null || echo '?')) - killing it manually to continue; T4 will flag this"
    kill "$LEFTOVER" 2>/dev/null || true
    sleep 0.6
fi

# --- T2: cross-pane drag release -----------------------------------------
step "T2: split, then drag L->R; release bytes must land in the LEFT pane"
xdotool key ctrl+shift+e
sleep 0.6
ok=""
for _ in $(seq 1 20); do
    [ "$("$CTL" list --json | grep -c '"id": ')" = "2" ] && { ok=1; break; }
    sleep 0.3
done
[ -n "$ok" ] || { "$CTL" list; fail "split did not create a second pane"; }
PANE2=$("$CTL" list --json | grep -o '"id": [0-9]\+' | grep -o '[0-9]\+$' | grep -v '^1$' | head -1)
echo "second pane id: $PANE2"

# Tracker in the LEFT pane: enable 1002 (button-event) + 1006 (SGR), then
# log raw pty input. stty raw: canonical mode would line-buffer the SGR
# reports (they carry no newline) and cat would never see them.
TRACK="printf '\\e[?1002h\\e[?1006h'; stty raw -echo; cat -v > $ROOT/pane1.log"
"$CTL" send agent1 --text "$TRACK"$'\n' >/dev/null
ok=""
for _ in $(seq 1 20); do [ -f "$ROOT/pane1.log" ] && { ok=1; break; }; sleep 0.25; done
[ -n "$ok" ] || { "$CTL" capture agent1; tail "$ROOT/app.log"; fail "tracker not running (pane1.log missing)"; }
sleep 0.8   # settle after the mode switches

# Press in the LEFT pane, drag across cells into the RIGHT pane, release.
xdotool mousemove "$LX" "$MY"
sleep 0.3
xdotool mousedown 1
for i in 1 2 3 4 5; do
    xdotool mousemove $((LX + (RX - LX) * i / 5)) "$MY"
    sleep 0.15
done
xdotool mouseup 1     # pointer is now over the RIGHT pane
sleep 0.6

# cat -v renders ESC as ^[ : press  ^[[<0;COL;ROWM   (capital M)
#                          motion ^[[<32;COL;ROWM   (32 = btn1 motion)
#                          release ^[[<0;COL;ROWm   (lowercase m)  <- P0
dump_tracker() { echo "--- pane1.log ---"; cat "$ROOT/pane1.log" 2>/dev/null; echo "---"; }
grep -q '\^\[\[<0;[0-9][0-9]*;[0-9][0-9]*M' "$ROOT/pane1.log" \
    || { dump_tracker; fail "SGR press missing in left pane"; }
grep -q '\^\[\[<32;[0-9][0-9]*;[0-9][0-9]*M' "$ROOT/pane1.log" \
    || { dump_tracker; fail "SGR motion missing in left pane"; }
grep -q '\^\[\[<0;[0-9][0-9]*;[0-9][0-9]*m' "$ROOT/pane1.log" \
    || { dump_tracker; fail "SGR RELEASE missing: drag ended over the right pane but the left pane owns the grab"; }
echo "tracker saw $(grep -o '\^\[\[<[0-9]*;[0-9]*;[0-9]*M' "$ROOT/pane1.log" | wc -l) press/motion reports and $(grep -o '\^\[\[<[0-9]*;[0-9]*;[0-9]*m' "$ROOT/pane1.log" | wc -l) release:"
grep -o '\^\[\[<3[0-9];[0-9]*;[0-9]*M' "$ROOT/pane1.log" | tail -2 | sed 's/^/    /'

# --- T3: scrollback keys --------------------------------------------------
step "T3: Shift+PageUp pages back, Shift+End returns live (pane $PANE2)"
xdotool mousemove "$RX" "$MY" click 1   # focus the right pane
sleep 0.4
"$CTL" send "$PANE2" --text "seq 1 500"$'\n' >/dev/null
wait_grep "$PANE2" '500' 40 \
    || { "$CTL" capture "$PANE2"; fail "seq tail (500) not visible in pane $PANE2"; }
xdotool key shift+Prior
sleep 0.4
if "$CTL" capture "$PANE2" | grep -q '500'; then
    "$CTL" capture "$PANE2"
    fail "Shift+PageUp did not page the viewport off the tail"
fi
"$CTL" capture "$PANE2" | grep -Eq '^[0-9]{1,3}$' \
    || { "$CTL" capture "$PANE2"; fail "paged view shows no seq lines"; }
xdotool key shift+End
sleep 0.4
"$CTL" capture "$PANE2" | grep -q '500' \
    || { "$CTL" capture "$PANE2"; fail "Shift+End did not return the viewport to live"; }

# --- T4: Ctrl+C must INTERRUPT the foreground job -------------------------
step "T4: Ctrl+C interrupts a foreground job (SIGINT, not just the echo)"
xdotool mousemove "$RX" "$MY" click 1   # focus pane 2
sleep 0.4
"$CTL" send "$PANE2" --text "sleep 45"$'\n' >/dev/null
sleep 0.8
P2=$(pane_pid "$PANE2")
[ -n "$P2" ] || fail "T4 setup: could not read pane $PANE2 pid from list --json"
JOB=$(kid_of "$P2" | head -1)
[ -n "$JOB" ] || fail "T4 setup: no foreground job in pane $PANE2"
xdotool key ctrl+c
INTERRUPTED=""
for _ in $(seq 1 40); do          # up to ~10s
    [ -z "$(kid_of "$P2")" ] && { INTERRUPTED=1; break; }
    sleep 0.25
done
if [ -z "$INTERRUPTED" ]; then
    "$CTL" capture "$PANE2"
    echo "pane-child SigIgn mask: $(awk '/SigIgn/{print $2}' /proc/"$P2"/status 2>/dev/null)"
    fail "GENUINE APP BUG: Ctrl+C echoed ^C but never interrupted 'sleep 45'. This exact failure shape was the inherited-SIG_IGN bug: apps launched in the background (shell &, desktop launchers) leave SIGHUP/INT/QUIT/TERM ignored, and SIG_IGN survives fork+execve; the fix resets SIGHUP/SIGINT/SIGQUIT/SIGTERM to SIG_DFL in the pty child branch of crates/vt-pane/src/pty.rs (next to the execve). If it recurs now, that reset was removed or bypassed - check the pane-child SigIgn mask printed above (from /proc/<pane-child>/status) and pty.rs."
fi
echo "foreground job ended within the poll window"

echo "ALL MOUSE/KEY E2E CHECKS PASSED"
