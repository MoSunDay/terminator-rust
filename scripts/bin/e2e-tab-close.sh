#!/usr/bin/env bash
# Real pointer regression for tab confirmation and pane-header direct close.
# Uses a private HOME, Xvfb and openbox; never touches the desktop session.
set -euo pipefail
source "$(dirname "$0")/e2e-lib.sh"
cd "$(dirname "$0")/../.."

ROOT=$(mktemp -d /tmp/term-e2e-tab-close-XXXXXX)
APP="${CARGO_TARGET_DIR:-target}/debug/terminator-rust"
CTL="${CARGO_TARGET_DIR:-target}/debug/terminator-ctl"
APP_PID=""
XVFB_PID=""
OPENBOX_PID=""
WID=""
E2E_FAIL_LOG="$ROOT/app.log"
cleanup() { e2e_cleanup "$APP_PID" "$OPENBOX_PID" "$XVFB_PID"; }
trap cleanup EXIT

# Persisted titles prove which tab closed, including after indices shift.
titles() {
    python3 - "$STATE" <<'PY'
import json, sys
windows = json.load(open(sys.argv[1]))["windows"]
print("|".join(",".join(t["title"] for t in w["tabs"]) for w in windows))
PY
}

wait_titles() {
    local want=$1 got=""
    for _ in $(seq 1 40); do
        got=$(titles)
        [ "$got" = "$want" ] && return 0
        sleep 0.25
    done
    fail "tabs: got '$got', want '$want'"
}

pane_count() {
    python3 - "$STATE" <<'PY'
import json, sys
root = json.load(open(sys.argv[1]))["windows"][0]["tabs"][0]["root"]
def count(node):
    return 1 if "Pane" in node else count(node["Split"]["first"]) + count(node["Split"]["second"])
print(count(root))
PY
}

wait_live_panes() {
    local want=$1 got=""
    for _ in $(seq 1 40); do
        got=$("$CTL" list --json 2>/dev/null | grep -c '"id": ' || true)
        [ "$got" = "$want" ] && return 0
        sleep 0.25
    done
    fail "live panes: got '$got', want '$want'"
}

confirm_tab() {
    geo
    # The shared modal is centered and 354x138 at the default font size.
    # Its right-hand 96px action button is 60px left of the frame edge.
    click_at $((X+WIDTH/2+117)) $((Y+HEIGHT/2+43))
}

click_first_tab_x() {
    geo
    # Preset tabname-NN chips are 124px wide at the default font size.
    click_at $((X+117)) $((Y+18))
}

step "build and launch a private five-tab window"
cargo build -p app -p ctl --bins --quiet
e2e_sandbox /bin/sh 1
e2e_preset_tabs "$STATE" 5
e2e_start_xvfb 1400x900x24
openbox >"$ROOT/openbox.log" 2>&1 & OPENBOX_PID=$!
sleep 1
"$APP" >"$ROOT/app.log" 2>&1 & APP_PID=$!
for _ in $(seq 1 40); do [ -S "$SOCK" ] && break; sleep 0.25; done
[ -S "$SOCK" ] || fail "control socket did not appear"
WID=$(wait_win '^terminator-rust$') || fail "root window did not appear"
sleep 1
activate
geo
[ "$WIDTH" -eq 1200 ] && [ "$HEIGHT" -eq 800 ] || fail "unexpected window size"
wait_titles 'tabname-01,tabname-02,tabname-03,tabname-04,tabname-05'

step "tab X opens confirmation; Escape keeps every tab"
click_first_tab_x
if [ "${E2E_KEEP:-0}" = "1" ]; then scrot "$ROOT/tab-modal.png"; fi
wait_titles 'tabname-01,tabname-02,tabname-03,tabname-04,tabname-05'
kill -0 "$APP_PID" || fail "tab X quit the app before confirmation"
xdotool key --clearmodifiers Escape
sleep 0.3
wait_titles 'tabname-01,tabname-02,tabname-03,tabname-04,tabname-05'

step "confirm closes only the requested tab"
click_first_tab_x
confirm_tab
wait_titles 'tabname-02,tabname-03,tabname-04,tabname-05'

step "middle-click on a tab also requires confirmation"
geo
park $((X+45)) $((Y+18))
xdotool click 2
sleep 0.3
wait_titles 'tabname-02,tabname-03,tabname-04,tabname-05'
confirm_tab
wait_titles 'tabname-03,tabname-04,tabname-05'

step "a multi-pane tab confirmation ends both panes"
xdotool key --clearmodifiers ctrl+shift+e
for _ in $(seq 1 40); do
    [ "$(pane_count)" -eq 2 ] && break
    sleep 0.25
done
[ "$(pane_count)" -eq 2 ] || fail "the first tab did not split"
geo
park $((X+45)) $((Y+18))
xdotool click 2
sleep 0.3
wait_titles 'tabname-03,tabname-04,tabname-05'
if [ "${E2E_KEEP:-0}" = "1" ]; then scrot "$ROOT/multi-pane-modal.png"; fi
confirm_tab
wait_titles 'tabname-04,tabname-05'
wait_live_panes 2

step "pane-header X closes its pane immediately"
geo
click_at $((X+WIDTH-12)) $((Y+48))
wait_titles 'tabname-05'
wait_live_panes 1

step "secondary window tab X confirms and closes only that window"
activate
xdotool key --clearmodifiers ctrl+shift+n
SEC_WID=$(wait_win 'terminator-rust #') || fail "secondary window did not appear"
WID=$SEC_WID
sleep 1
activate
geo
# The seed tab is titled 'shell', making its close glyph center X+70.
click_at $((X+70)) $((Y+18))
wait_titles 'tabname-05|shell'
wait_live_panes 2
kill -0 "$APP_PID" || fail "secondary tab X quit the app"
confirm_tab
wait_titles 'tabname-05'
wait_live_panes 1
WID=$(wait_win '^terminator-rust$') || fail "root window disappeared"

step "the last tab waits for confirmation, then closes the app"
activate
click_first_tab_x
wait_titles 'tabname-05'
kill -0 "$APP_PID" || fail "last tab X quit before confirmation"
confirm_tab
wait_pid_gone "$APP_PID" 40 || fail "confirmed last tab did not quit"
APP_PID=""
echo "tab confirmation, cancellation, pane direct close and window isolation: OK"
