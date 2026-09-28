#!/usr/bin/env bash
# Isolated X11: tmux -> OSC52 -> OS clipboard and local modifier-click browser.
set -euo pipefail
ROOT=$(mktemp -d)
E2E_FAIL_LOG="$ROOT/app.log"
source "$(dirname "$0")/e2e-lib.sh"
cd "$(dirname "$0")/../.."
APP="${E2E_APP:-${CARGO_TARGET_DIR:-target}/release/terminator-rust}"
CTL="${CARGO_TARGET_DIR:-target}/release/terminator-ctl"
APP=$(realpath "$APP"); CTL=$(realpath "$CTL")
TMUX_NAME="clipboard-e2e-$$"
APP_PID=""; XVFB_PID=""
cleanup() { tmux -L "$TMUX_NAME" capture-pane -p > "$ROOT/tmux-capture" 2>/dev/null || true; tmux -L "$TMUX_NAME" kill-server 2>/dev/null || true; e2e_cleanup "$APP_PID" "$XVFB_PID"; }
trap cleanup EXIT
# Build separately before running; this test never touches a live desktop.
e2e_sandbox /bin/bash 0
export BROWSER="$ROOT/browser-probe"
cat > "$BROWSER" <<'MOCK'
#!/bin/sh
printf '%s' "$1" > "$HOME/browser-url"
MOCK
chmod +x "$BROWSER"
e2e_start_xvfb 1200x800x24
RUST_LOG=terminator_rust::input=debug setsid "$APP" >"$ROOT/app.log" 2>&1 < /dev/null & APP_PID=$!
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
send() { "$CTL" send "$ID" --text "$1"$'\n' >/dev/null; sleep .5; }
send "tmux -L $TMUX_NAME -f /data00/viking-proxy/codex-screen-socks5/codex_tmux.conf new-session -s probe"
tmux -L "$TMUX_NAME" source-file /data00/viking-proxy/tmux-wheel.conf
tmux -L "$TMUX_NAME" show-options -s user-keys > "$ROOT/tmux-options"
tmux -L "$TMUX_NAME" set -s set-clipboard on
step 'tmux load-buffer -w arrives in the outer OS clipboard'
printf '%s' 'clipboard-through-tmux' | tmux -L "$TMUX_NAME" load-buffer -w -
sleep .5
wait_clipboard 'clipboard-through-tmux' || fail 'tmux clipboard missing'
step 'application OSC52 through tmux arrives in the OS clipboard'
send "printf '\\033]52;c;ZGlyZWN0LWFwcGxpY2F0aW9u\\007'"
wait_clipboard 'direct-application' || fail 'application clipboard missing'
step 'copy shortcut reaches the TUI through tmux and returns clipboard content'
cat > "$ROOT/copy-tui.py" <<'PYCODE'
import os, tty, termios, base64, select
modifier = b"9"
old = termios.tcgetattr(0)
try:
    tty.setraw(0)
    os.write(1, b"\x1b[>1u\x1b[?1002h\x1b[?1006hCOPY-READY")
    data = b""
    while len(data) < 64 and select.select([0], [], [], 3)[0]:
        data += os.read(0, 1)
        if data.endswith(b"u"):
            break
    open(__file__ + ".received", "wb").write(data)
    if data == b"\x1b[99;" + modifier + b"u":
        os.write(1, b"\x1b]52;c;" + base64.b64encode(b"copied-by-tui-shortcut") + b"\x07")
    else:
        os.write(1, repr(data).encode())
finally:
    os.write(1, b"\x1b[<u\x1b[?1002l\x1b[?1006l")
    termios.tcsetattr(0, termios.TCSANOW, old)
PYCODE
send "COPY_PROBE_MOD=${COPY_PROBE_MOD:-6} python3 $ROOT/copy-tui.py"
xdotool windowfocus "$WID"
xdotool key ctrl+shift+c
sleep 3.5
wait_clipboard 'copied-by-tui-shortcut' || fail 'copy shortcut did not reach TUI'
step 'Ctrl-click opens a visible URL locally while mouse tracking is on'
URL='https://example.com/verify?flow_id=probe&user_code=PAFP-N8YK'
send "printf '\\033[2J\\033[H$URL\\r\\n\\033[?1002h\\033[?1006h'"
xdotool mousemove $((X+90)) $((Y+70)) keydown ctrl
sleep .1
xdotool click 1 keyup ctrl
sleep .5
[ "$(cat "$HOME/browser-url" 2>/dev/null)" = "$URL" ] || fail 'local browser did not receive full URL'
step 'PASS: clipboard forwarding and local link opening'
