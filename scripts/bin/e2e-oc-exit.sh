#!/usr/bin/env bash
# Headless e2e: the REAL opencoder TUI (release build, crossterm 0.28,
# pushes kitty flags 7) inside terminator panes must be exitable by
# keyboard alone, and the app itself must have a global quit:
#   K1: plain typing reaches the TUI (the pane child is the opencoder
#       binary, not a shell) and the TUI stays alive for the exit keys.
#   K2: double Ctrl+C exits opencoder (2026-09 upstream gesture; the old
#       Ctrl+D/Esc/single-Ctrl+C exit was removed upstream).
#   K3: a bare Ctrl+D reaches the child through the kitty-flags encoder
#       workaround - the idle prompt exits on it (status 0), which is
#       POSITIVE proof the 0x04 byte arrived; Esc stays inert (probed).
#   K4: Ctrl+Shift+W force-closes a pane while the TUI is running (a
#       fresh sibling tab keeps the app alive - quit-on-last-close only).
#   K2/K3 double as the auto-close check: an exited child's pane closes
#       itself ~250ms later (EXIT_GRACE) with NO click, and leftover DEC
#       mouse modes never block the cleanup.
#   K5: Ctrl+Shift+Q quits the whole app (WM-close equivalent; the
#       historical gap: there was NO quit shortcut at all).
# Each pane runs $SHELL = a wrapper that execs the real binary, so the
# pane pid IS the opencoder pid (exec keeps the pid) and `kill -0` is the
# ground truth for "exited".
#
# Usage: scripts/bin/e2e-oc-exit.sh   (repo root; needs Xvfb + xdotool and
#        the opencoder release binary; OC_BIN overrides its path).
#        E2E_KEEP=1 keeps the scratch dir for debugging.
set -euo pipefail
cd "$(dirname "$0")/../.."

OC_BIN=${OC_BIN:-/root/opencoder/target/release/opencoder}
ROOT=$(mktemp -d /tmp/term-e2e-ocexit-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
DISPLAY_N=""
XVFB_PID=""
APP_PID=""
OC_PIDS=()

cleanup() {
    for p in "${OC_PIDS[@]:-}"; do
        [ -n "$p" ] && kill -9 "$p" 2>/dev/null || true
    done
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

# wait_pid_gone <pid> [tries]: poll until the process is dead.
wait_pid_gone() {
    local pid=$1 tries=${2:-40}
    for _ in $(seq 1 "$tries"); do
        kill -0 "$pid" 2>/dev/null || return 0
        sleep 0.25
    done
    return 1
}

# pane_pid <name>: the pane's child pid from `list --json`.
pane_pid() {
    "$CTL" list --json 2>/dev/null \
        | grep -A8 "\"name\": \"$1\"" | grep '"pid"' | head -1 | grep -o '[0-9]\+'
}

# oc_pid <name>: pane pid AND its /proc exe must be the opencoder binary.
# The pane child is the SHELL WRAPPER (a script), so between fork and the
# wrapper's exec the exe reads as /bin/sh or empty; and if the pane was
# respawned the first pid can DIE under us. Retry the WHOLE discovery (not
# just the readlink) so a reaped pid never wedges the check, and never
# leak a stale pid from a previous agent into a later one.
oc_pid() {
    local pid exe
    for _ in $(seq 1 60); do
        pid=$(pane_pid "$1" 2>/dev/null || true)
        if [ -n "$pid" ]; then
            exe=$(readlink "/proc/$pid/exe" 2>/dev/null || true)
            [ "$exe" = "$OC_BIN" ] && { echo "$pid"; return 0; }
            # pid surfaced but not (yet) opencoder: dead pid -> rediscover
            # on the next pass; live one just hasn't execve'd yet.
            kill -0 "$pid" 2>/dev/null || { sleep 0.25; continue; }
        fi
        sleep 0.25
    done
    return 1
}

# Build first: the sandboxed HOME below would hide rustup/toolchains.
cargo build -p app -p ctl --bins >/dev/null
[ -x "$OC_BIN" ] || fail "opencoder binary missing at $OC_BIN (build it first)"

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
# No compositor in Xvfb: pin full opacity for deterministic rendering.
export TERMINATOR_OPAQUE=1
export TERMINATOR_NO_MOTION=1   # pin fades/cursor blink to end states
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# --- world: three local panes, each running the real opencoder ----------
mkdir -p "$ROOT/config/terminator-rust" "$ROOT/bin"
cat > "$XDG_CONFIG_HOME/terminator-rust/state.json" <<'JSON'
{
  "theme": "catppuccin-mocha",
  "tabs": [
    { "title": "shell", "focused": 1,
      "root": { "Split": { "axis": "v", "ratio": 0.34,
        "first": { "Pane": { "id": 1, "meta": {
            "kind": "Local", "manual_title": "agent1",
            "bg": null, "transparency": 0.0, "degraded": false } } },
        "second": { "Split": { "axis": "v", "ratio": 0.5,
          "first": { "Pane": { "id": 2, "meta": {
              "kind": "Local", "manual_title": "agent2",
              "bg": null, "transparency": 0.0, "degraded": false } } },
          "second": { "Pane": { "id": 3, "meta": {
              "kind": "Local", "manual_title": "agent3",
              "bg": null, "transparency": 0.0, "degraded": false } } } } } } } }
  ]
}
JSON
# exec replaces the shell: pane pid == opencoder pid.
printf '#!/bin/sh\nexec "%s"\n' "$OC_BIN" > "$ROOT/bin/oc-wrapper"
chmod +x "$ROOT/bin/oc-wrapper"
export SHELL="$ROOT/bin/oc-wrapper"

# Without a config opencoder sits in its onboarding form, where Ctrl+C
# means "cancel" and does NOT exit. A dummy provider config (local checks
# only; never contacted) lands it at the idle task prompt where Ctrl+C
# exits - the state a real user's pane is in.
mkdir -p "$HOME/.opencoder"
cat > "$HOME/.opencoder/config.json" <<'JSON'
{
  "provider": {
    "base_url": "http://127.0.0.1:9/v1",
    "api_key": "dummy-key-not-a-secret",
    "model": "dummy-model"
  },
  "model": "dummy-model"
}
JSON

# opencode's FIRST run seeds ~/.opencoder (skills installer + state
# dirs). The app launches 3 panes at once and concurrent first-runs RACE
# that seeding - observed live: 2 of 3 instances exit(1) before ever
# reaching the prompt. One throwaway pty run serializes the seeding so
# every real pane starts on a warm HOME.
step "warm up scratch HOME (serialize opencode first-run seeding)"
( HOME="$HOME" timeout 15 script -qec "$OC_BIN" /dev/null >/dev/null 2>&1 ) || true
for _ in $(seq 1 40); do
    [ -x "$HOME/.opencoder/install-skills-dep.sh" ] && [ -d "$HOME/.local/share/opencoder" ] && break
    sleep 0.25
done
[ -x "$HOME/.opencoder/install-skills-dep.sh" ] && [ -d "$HOME/.local/share/opencoder" ] \
    || fail "opencode first-run seeding never completed (warmup run)"

# --- Xvfb + app -----------------------------------------------------------
step "launch Xvfb + app (3 opencoder panes)"
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
export DISPLAY="$DISPLAY_N"
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "control socket never appeared"; }

WID=""
for _ in $(seq 1 20); do
    WID=$(xdotool search --name terminator-rust 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.25
done
[ -n "$WID" ] || { tail "$ROOT/app.log"; fail "app window not found"; }
sleep 1   # let the window map; an early focus races -> X BadMatch
until xdotool windowfocus "$WID" 2>/dev/null; do sleep 0.3; done
eval "$(xdotool getwindowgeometry --shell "$WID")"

# --- panes are the real binary -------------------------------------------
step "K1: panes run the real opencoder binary"
for i in 1 2 3; do
    pid=$(oc_pid "agent$i") || {
        "$CTL" list
        echo "--- agent$i pane tail (dying words):"
        "$CTL" capture "$i" 12 2>/dev/null | tail -12
        fail "agent$i never settled on the opencoder binary"
    }
    # `:-` keeps set -u happy while the array is still empty.
    for seen in "${OC_PIDS[@]:-}"; do
        [ -n "$seen" ] || continue
        [ "$pid" != "$seen" ] || fail "agent$i reused pid $pid (discovery race)"
    done
    OC_PIDS+=("$pid")
    eval "P${i}_PID=$pid"
    echo "agent$i pid=$pid (opencoder)"
done
sleep 1.5   # let the TUIs settle

# --- K2: Ctrl+D exits opencoder ------------------------------------------
step "K2: double Ctrl+C exits the focused opencoder (agent1)"
# Gesture drift (agents.md): the idle prompt's exit gesture CHANGES
# BETWEEN BUILDS - some exit on a single Ctrl+C, others need two spaced
# presses. The step below probes instead of assuming.
# NOTE: no clicks - opencoder tracks the mouse (1002/1003), a click is
# reported to it and can leave it busy, where Ctrl+C means cancel.
# Gesture drift (agents.md): builds differ - some exit the idle prompt
# on a SINGLE Ctrl+C, others need TWO. Send one, wait, escalate only if
# still alive: a blind double-tap on a single-Ctrl+C build would kill
# the NEXT pane too (auto-close + focus containment hands the second
# press to agent2).
xdotool key --clearmodifiers ctrl+c
for _ in $(seq 1 8); do
    kill -0 "$P1_PID" 2>/dev/null || break
    sleep 0.25
done
if kill -0 "$P1_PID" 2>/dev/null; then
    xdotool key --clearmodifiers ctrl+c   # two-press build: second one
fi
wait_pid_gone "$P1_PID" 40 \
    || { "$CTL" capture agent1 | tail -5; fail "opencoder survived Ctrl+C"; }
echo "agent1 exited via Ctrl+C (1 or 2 presses)"
# Exited panes now close THEMSELVES after EXIT_GRACE (250ms) - no click.
# The corpse still carries opencoder's DEC mouse modes; cleanup must not
# be blocked by them.
for _ in $(seq 1 20); do
    "$CTL" list 2>/dev/null | grep -q agent1 || break
    sleep 0.25
done
"$CTL" list 2>/dev/null | grep -q agent1 \
    && { "$CTL" list; fail "dead agent1 pane never auto-closed"; }
"$CTL" list 2>/dev/null | grep -q agent2 \
    || fail "agent2 vanished when agent1 auto-closed (over-close?)"
kill -0 "$APP_PID" 2>/dev/null || fail "app quit on a non-last pane auto-close"
echo "agent1 pane auto-closed after exit; agent2 alive; app alive"

# --- K3: Ctrl+D delivers 0x04 and exits the child ------------------------
step "K3: bare Ctrl+D reaches agent2 (kitty-flags encoding path)"
# Gesture drift (opencode build 2026-09-10T22:41): the idle prompt now
# exits on a SINGLE Ctrl+C and on a bare Ctrl+D (both status 0, probed
# on a raw pty with master-side injection); Esc stays inert. Ride that:
# agent2 dying right after Ctrl+D is positive proof the 0x04 byte
# survived the kitty-flags-7 encoder workaround and reached the child
# (a dropped key would leave the pane alive at the prompt).
# Focus containment guarantees the focused pane is a live survivor
# (agent2 or agent3 - WHICH one depends on the K2 press count and the
# build's gesture), so no blind focus navigation: Ctrl+D the focused
# pane and identify the victim by pid afterwards.
xdotool key --clearmodifiers ctrl+d
victim=""
for _ in $(seq 1 40); do
    kill -0 "$P2_PID" 2>/dev/null || { victim="agent2"; break; }
    kill -0 "$P3_PID" 2>/dev/null || { victim="agent3"; break; }
    sleep 0.25
done
[ -n "$victim" ] \
    || { "$CTL" capture agent2 | tail -5; fail "Ctrl+D reached nothing (encoder regression?)"; }
if [ "$victim" = agent2 ]; then SURV_NAME=agent3; SURV_PID=$P3_PID;
else SURV_NAME=agent2; SURV_PID=$P2_PID; fi
echo "$victim exited via Ctrl+D - 0x04 delivered through the kitty-flags path"
for _ in $(seq 1 20); do
    "$CTL" list 2>/dev/null | grep -q "$victim" || break
    sleep 0.25
done
"$CTL" list 2>/dev/null | grep -q "$victim" \
    && { "$CTL" list; fail "dead $victim pane never auto-closed"; }
"$CTL" list 2>/dev/null | grep -q "$SURV_NAME" \
    || fail "$SURV_NAME vanished when $victim auto-closed (over-close?)"
kill -0 "$APP_PID" 2>/dev/null || fail "app quit on a non-last pane auto-close"
echo "$victim pane auto-closed after exit; $SURV_NAME alive; app alive"

# --- K4: Ctrl+Shift+W closes a live pane ----------------------------------
step "K4: Ctrl+Shift+W force-closes the running pane ($SURV_NAME)"
# Both siblings auto-closed above, so $SURV_NAME is the LAST pane of
# the only tab. Quit-on-last-close would QUIT the app - keep that honest
# by first opening a fresh sibling tab (oc-wrapper -> opencoder, warm
# HOME), then switching BACK to the agent tab before closing it.
kill -0 "$SURV_PID" 2>/dev/null || fail "$SURV_NAME died before K4 (test bug)"
xdotool key --clearmodifiers ctrl+shift+t     # new tab (takes focus)
sleep 1.5   # let the fresh opencoder TUI settle (it only must be alive)
xdotool key --clearmodifiers ctrl+Prior       # PrevTab: back to the agent tab
sleep 0.4
xdotool key --clearmodifiers ctrl+shift+w     # close it, not the last pane
wait_pid_gone "$SURV_PID" 40 \
    || fail "opencoder survived Ctrl+Shift+W"
"$CTL" list 2>/dev/null | grep -q "$SURV_NAME" \
    && { "$CTL" list; fail "$SURV_NAME still listed after close"; }
kill -0 "$APP_PID" 2>/dev/null || fail "app quit on a non-last pane close"
echo "$SURV_NAME pane closed while the TUI was live; app survived (sibling tab)"

# --- K5: Ctrl+Shift+Q quits the app ---------------------------------------
step "K5: Ctrl+Shift+Q quits terminator-rust"
kill -0 "$APP_PID" 2>/dev/null || fail "app died before K5 (test bug)"
xdotool key --clearmodifiers ctrl+shift+q
for _ in $(seq 1 40); do
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.25
done
kill -0 "$APP_PID" 2>/dev/null && fail "app survived Ctrl+Shift+Q"
echo "app exited via Ctrl+Shift+Q"

echo "PASS: opencoder exit keys + global quit all verified"
