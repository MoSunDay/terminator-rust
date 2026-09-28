#!/usr/bin/env bash
# Extend the isolated notice GUI checks to Codex-style detached hooks.
set -euo pipefail
source "$(dirname "$0")/e2e-notice.sh"
ADAPTER="$(pwd)/scripts/bin/codex-notice.py"
TEST_TMUX=""
cleanup() {
    [ -z "$TEST_TMUX" ] || tmux -L "$TEST_TMUX" kill-server 2>/dev/null || true
    e2e_cleanup "$APP_PID" "$XVFB_PID" $KEEPER_PIDS
}

step "C1: detached hook inside keeper -> background badge"
click_alpha
"$CTL" send "$BETA_ID" --text "setsid -w /usr/bin/python3 $ADAPTER >$ROOT/hook.stdout 2>$ROOT/hook.stderr"$'\n' >/dev/null
sleep 1.5
shot "$ROOT/c1.png"
dot_scan "$ROOT/c1.png" expect
[ ! -s "$ROOT/hook.stdout" ] || fail "hook polluted captured stdout"
[ ! -s "$ROOT/hook.stderr" ] || { cat "$ROOT/hook.stderr"; fail "hook error"; }
click_beta_badged
shot "$ROOT/c1-ack.png"
dot_scan "$ROOT/c1-ack.png" forbid
echo "PASS C1: detached hook reached ancestor tty; badge acknowledged"

step "C2: detached hook inside tmux inside keeper -> background badge"
TEST_TMUX="terminator-notice-e2e-$$"
# The keeper's shell is restored by exiting only this isolated tmux pane.
printf 'set -g allow-passthrough on\nset -g status off\n' > "$ROOT/tmux.conf"
"$CTL" send "$BETA_ID" --text "tmux -L $TEST_TMUX -f $ROOT/tmux.conf new-session -s verify"$'\n' >/dev/null
sleep 1.5
click_alpha
"$CTL" send "$BETA_ID" --text "setsid -w /usr/bin/python3 $ADAPTER >$ROOT/tmux-hook.stdout 2>$ROOT/tmux-hook.stderr"$'\n' >/dev/null
sleep 1.5
shot "$ROOT/c2.png"
dot_scan "$ROOT/c2.png" expect
[ ! -s "$ROOT/tmux-hook.stdout" ] || fail "tmux hook polluted captured stdout"
[ ! -s "$ROOT/tmux-hook.stderr" ] || { cat "$ROOT/tmux-hook.stderr"; fail "tmux hook error"; }
click_beta_badged
shot "$ROOT/c2-ack.png"
dot_scan "$ROOT/c2-ack.png" forbid
tmux -L "$TEST_TMUX" kill-server
echo "PASS C2: detached hook passed through tmux and keeper; badge acknowledged"
echo "e2e-codex-notice: ALL PASS"
