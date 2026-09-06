#!/usr/bin/env bash
# Headless e2e for the terminator-rust control channel (M1) and the `oc`
# submission path (M3). Everything runs inside this one invocation: Xvfb,
# the app, a fake `opencoder` process holding a fixture store open inside a
# named pane, then terminator-ctl drives it all over $TERMINATOR_SOCK.
#
# Usage: scripts/bin/e2e-ipc-oc.sh   (run from the repo root; needs Xvfb)
set -euo pipefail
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-XXXXXX)
APP=target/debug/terminator-rust
CTL=target/debug/terminator-ctl
FIX=target/debug/oc-store-fixture
DISPLAY_N=":$$"          # unique per run; no clash with parallel scripts
XVFB_PID=""
APP_PID=""

cleanup() {
    [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
    [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
    pkill -f "$ROOT/bin/opencoder" 2>/dev/null || true
    sleep 0.3
    rm -rf "$ROOT"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo "== $*"; }

# Build first: the sandboxed HOME below would hide rustup/toolchains.
cargo build -p app -p ctl -p oc-store --bins >/dev/null

export XDG_CONFIG_HOME="$ROOT/config"
export XDG_RUNTIME_DIR="$ROOT/runtime"
export HOME="$ROOT/home"
SOCK="$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"
export TERMINATOR_SOCK="$SOCK"
mkdir -p "$XDG_CONFIG_HOME/terminator-rust" "$XDG_RUNTIME_DIR" "$HOME"

# --- world -------------------------------------------------------------
mkdir -p "$ROOT/config/terminator-rust" "$ROOT/store" "$ROOT/bin"
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
SID="01HXE2ESESSION00001"
"$FIX" "$ROOT/store/opencoder.db" "$SID" >/dev/null
cat > "$ROOT/bin/opencoder" <<'SH'
#!/bin/bash
exec 9<"$1"
sleep 600
SH
chmod +x "$ROOT/bin/opencoder"

# --- app + Xvfb ----------------------------------------------------------
step "launch Xvfb $DISPLAY_N + app"
Xvfb "$DISPLAY_N" -screen 0 1200x800x24 & XVFB_PID=$!
sleep 0.7
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || { tail "$ROOT/app.log"; fail "control socket never appeared"; }
# capture/send is full remote control: the socket must be owner-only.
[ "$(stat -c %a "$SOCK")" = "600" ] || fail "socket perms $(stat -c %a "$SOCK") != 600"
sleep 2   # let the pane's shell boot

# --- M1: list / capture / send ------------------------------------------
step "list"
LIST=$("$CTL" list)
echo "$LIST"
echo "$LIST" | grep -q "agent1" || fail "list: named pane missing"

step "send + capture roundtrip"
MARK="hi-ipc-$$"
"$CTL" send agent1 --text "echo $MARK"$'\n' >/dev/null
for _ in $(seq 1 20); do
    "$CTL" capture agent1 | grep -q "$MARK" && break
    sleep 0.3
done
"$CTL" capture agent1 | grep -q "$MARK" || fail "capture: marker not on screen"

step "unknown pane errors"
if "$CTL" send nope --text x >/dev/null 2>&1; then fail "send to unknown pane should fail"; fi

step "TERMINATOR_SOCK inherited by pane child"
PANE_PID=$("$CTL" list --json | grep '"pid"' | head -1 | grep -o '[0-9]*')
tr '\0' '\n' < "/proc/$PANE_PID/environ" | grep -q "TERMINATOR_SOCK=$SOCK" \
    || fail "pane child lacks TERMINATOR_SOCK"

# --- M3: oc link / submit / status / wait --------------------------------
step "spawn fake opencoder inside the pane"
"$CTL" send agent1 --text "$ROOT/bin/opencoder $ROOT/store/opencoder.db &"$'\n' >/dev/null
sleep 1

step "oc link (discovery via /proc)"
"$CTL" oc link agent1 | tee /dev/stderr | grep -q "linked pane 'agent1'" \
    || fail "oc link output unexpected"

step "oc submit steer"
SUBMIT=$("$CTL" oc submit agent1 "fix the login crash" --delivery steer)
echo "$SUBMIT"
echo "$SUBMIT" | grep -q "#1 (steer)" || fail "submit seq/delivery wrong"

step "oc status shows pending"
STATUS=$("$CTL" oc status agent1)
echo "$STATUS"
echo "$STATUS" | grep -q '#1 steer adm=1 "fix the login crash"' || fail "pending row missing"

step "honest timeout while nothing consumes"
if "$CTL" oc submit agent1 "never lands" --delivery steer --wait 2 >/dev/null 2>&1; then
    fail "wait should time out honestly"
fi

step "consume + wait (as the TUI would)"
( "$CTL" oc submit agent1 "then run the tests" --delivery queue --wait 10 >"$ROOT/wait.out" 2>&1 ) &
SUB_PID=$!
sleep 2
QSEQ=$(grep -o '#[0-9]*' "$ROOT/wait.out" | head -1 | tr -d '#')
[ -n "$QSEQ" ] || { cat "$ROOT/wait.out"; fail "no seq parsed from submit output"; }
"$FIX" "$ROOT/store/opencoder.db" consume "$SID" "$QSEQ" queue >/dev/null
wait "$SUB_PID" || { cat "$ROOT/wait.out"; fail "wait-consume path failed"; }
grep -q "consumed after" "$ROOT/wait.out" || fail "wait did not report consumption"

step "oc status shows receipts"
"$CTL" oc status agent1 | tee /dev/stderr | grep -q "#$QSEQ queue_consumed" \
    || fail "receipt missing"

step "oc sessions"
"$CTL" oc sessions agent1 | tee /dev/stderr | grep -q "$SID" || fail "session list empty"

step "stale socket (SIGTERM leaves the file; next start reclaims it)"
kill "$APP_PID"; APP_PID=""
sleep 1   # SIGTERM: no destructors, the socket file is reclaimed on next start
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid "$APP" >"$ROOT/app2.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do "$CTL" list >/dev/null 2>&1 && break; sleep 0.25; done
"$CTL" list | grep -q "agent1" || { tail "$ROOT/app2.log"; fail "second instance could not reclaim the socket"; }
[ "$(stat -c %a "$SOCK")" = "600" ] || fail "reclaimed socket perms $(stat -c %a "$SOCK") != 600"
kill "$APP_PID"; APP_PID=""

echo "ALL E2E CHECKS PASSED"
