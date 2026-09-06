#!/usr/bin/env bash
# Headless e2e: the REAL opencoder TUI (release build, crossterm 0.28,
# pushes kitty flags 7) inside terminator panes must be exitable by
# keyboard alone, and the app itself must have a global quit:
#   K1: plain typing reaches the TUI (the pane child is the opencoder
#       binary, not a shell) and the TUI stays alive for the exit keys.
#   K2: Ctrl+D exits opencoder (every mode: menu / prompt / task).
#   K3: Ctrl+C exits opencoder when idle (busy = cancel only, by design).
#   K4: Ctrl+Shift+W force-closes a pane while the TUI is running.
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
DISPLAY_N=":$$"
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

# Build first: the sandboxed HOME below would hide rustup/toolchains.
cargo build -p app -p ctl --bins >/dev/null
[ -x "$OC_BIN" ] || fail "opencoder binary missing at $OC_BIN (build it first)"

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
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

# --- Xvfb + app -----------------------------------------------------------
step "launch Xvfb $DISPLAY_N + app (3 opencoder panes)"
Xvfb "$DISPLAY_N" -screen 0 1200x800x24 & XVFB_PID=$!
sleep 0.7
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
    for _ in $(seq 1 40); do
        pid=$(pane_pid "agent$i") && [ -n "$pid" ] && break
        sleep 0.25
    done
    [ -n "${pid:-}" ] || { "$CTL" list; fail "agent$i has no child pid"; }
    exe=$(readlink "/proc/$pid/exe" 2>/dev/null || true)
    [ "$exe" = "$OC_BIN" ] || fail "agent$i runs $exe, not opencoder"
    OC_PIDS+=("$pid")
    eval "P${i}_PID=$pid"
    echo "agent$i pid=$pid (opencoder)"
done
sleep 1.5   # let the TUIs settle

# --- K2: Ctrl+D exits opencoder ------------------------------------------
step "K2: Ctrl+D exits the focused opencoder (agent1)"
# NOTE: no clicks - opencoder tracks the mouse (1002/1003), a click is
# reported to it and can leave it busy, where Ctrl+C means cancel.
xdotool key --clearmodifiers ctrl+d
wait_pid_gone "$P1_PID" 40 \
    || { "$CTL" capture agent1 | tail -5; fail "opencoder survived Ctrl+D"; }
echo "agent1 exited via Ctrl+D"

# --- K3: Ctrl+C exits idle opencoder --------------------------------------
step "K3: Ctrl+C exits the idle opencoder (agent2)"
xdotool key --clearmodifiers ctrl+shift+Right   # focus agent2
sleep 0.4
xdotool key --clearmodifiers ctrl+c
wait_pid_gone "$P2_PID" 40 \
    || { "$CTL" capture agent2 | tail -5; fail "opencoder survived Ctrl+C"; }
echo "agent2 exited via Ctrl+C"

# --- K4: Ctrl+Shift+W closes a live pane ----------------------------------
step "K4: Ctrl+Shift+W force-closes the running pane (agent3)"
kill -0 "$P3_PID" 2>/dev/null || fail "agent3 died before K4 (test bug)"
xdotool key --clearmodifiers ctrl+shift+Right   # focus agent3
sleep 0.4
xdotool key --clearmodifiers ctrl+shift+w
wait_pid_gone "$P3_PID" 40 \
    || fail "opencoder survived Ctrl+Shift+W"
"$CTL" list 2>/dev/null | grep -q agent3 \
    && { "$CTL" list; fail "agent3 still listed after close"; }
echo "agent3 pane closed while the TUI was live"

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
