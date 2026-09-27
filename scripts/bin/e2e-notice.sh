#!/usr/bin/env bash
# Headless e2e for the tab attention bubble under the FINAL notice
# semantics: ONE transport. `terminator-ctl notice` takes NO arguments
# (any argument = usage error, rc!=0) and never talks to the app socket,
# the keeper, or the environment: it writes the canonical OSC 9 bytes
# (`\x1b]9;terminator-rust notice\x07`, from ipc_proto::notice_osc()) to
# /dev/tty (stdout fallback), best-effort, ALWAYS exit 0. The bytes are
# ordinary terminal OUTPUT, so the pane's VT parses them and its tab
# gets the badge - in EVERY pane kind. The keeper (terminator-session
# attach) is exercised as ONE case: the bytes traverse the attach stream
# (and thus the replay history) like any other PTY output. The
# zero-binary fallback is the same bytes via bare printf.
# Badge pixels: solid blue dot rgb(70,150,255), radius 5px at font 15,
# in an 18px strip that widens the chip; drawn only while the noticed
# tab is NOT the focused window's active tab - clicking the chip
# acknowledges (clears) it. Checks:
#   N1: notice typed into PLAIN pane 13 (background tab) -> badge pixels
#       RIGHT of the ACTIVE (alpha) chip; success is SILENT (capture:
#       command echoed exactly once, no rendered OSC garbage).
#   N2: notice again -> idempotent (BTreeSet): still ONE cluster.
#   N3: negative: `notice foo` -> usage error rc!=0, empty stdout,
#       badge unchanged; the bare command always exits 0.
#   N4: clicking the badged chip switches the tab (active_tab=1 in
#       state.json); the focused window acks its active tab -> gone.
#   N5: notice while beta IS the focused active tab -> suppressed.
#   N6: background beta again -> notice re-badges at the same x.
#   N7: zero-binary fallback: printf '\033]9;terminator-rust notice\007'
#       typed into the plain pane -> badge at the same x; ack clears.
#   N8: keeper: pane 13 attaches to "notes" (private socket + list
#       STATE=attached), then notice INSIDE the keeper shell -> badge
#       (bytes traverse the keeper); ack clears.
#   N9: app + keeper still answer.
# Requires: Xvfb, xdotool, scrot, PIL (python3-PIL). E2E_KEEP=1 keeps
# the scratch dir.
set -euo pipefail
ROOT=$(mktemp -d)
E2E_FAIL_LOG="$ROOT/app.log"
source "$(dirname "$0")/e2e-lib.sh"
cd "$(dirname "$0")/../.."
APP="${CARGO_TARGET_DIR:-target}/debug/terminator-rust"
CTL="${CARGO_TARGET_DIR:-target}/debug/terminator-ctl"
TS="${CARGO_TARGET_DIR:-target}/debug/terminator-session"
BINDIR=$(cd "$(dirname "$TS")" && pwd)   # absolute: panes PATH-export it
APP_PID=""; XVFB_PID=""; KEEPER_PIDS=""

cleanup() { e2e_cleanup "$APP_PID" "$XVFB_PID" $KEEPER_PIDS; }
trap cleanup EXIT
# Build first (the sandboxed HOME below would hide rustup/toolchains),
# then bash panes + session restore: the preset below must load.
cargo build -p app -p ctl -p terminator-session --bins >/dev/null
e2e_sandbox /bin/bash 1

cat > "$STATE" <<'JSON'
{
  "theme": "dracula",
  "settings": { "split_axis": "v", "split_ratio": 0.5 },
  "windows": [
    { "id": 1, "active_tab": 0, "tabs": [
      { "title": "alpha", "focused": 11,
        "root": { "Split": { "axis": "v", "ratio": 0.5,
          "first":  { "Pane": { "id": 11, "meta": { "kind": "Local", "manual_title": null, "bg": null, "transparency": 0.0, "degraded": false } } },
          "second": { "Pane": { "id": 12, "meta": { "kind": "Local", "manual_title": null, "bg": null, "transparency": 0.0, "degraded": false } } } } } },
      { "title": "beta", "focused": 13,
        "root": { "Pane": { "id": 13, "meta": { "kind": "Local", "manual_title": "notes", "bg": null, "transparency": 0.0, "degraded": false } } } }
    ] }
  ]
}
JSON

# --- Xvfb + app ----------------------------------------------------------
step "launch Xvfb + app"
e2e_start_xvfb 1200x800x24
env DISPLAY="$DISPLAY_N" RUST_LOG=info setsid nohup "$APP" </dev/null >"$ROOT/app.log" 2>&1 & APP_PID=$!
SOCK_UP=""
for _ in $(seq 1 40); do [ -S "$SOCK" ] && { SOCK_UP=1; break; }; sleep 0.25; done
[ -n "$SOCK_UP" ] || { tail -20 "$ROOT/app.log" || true; fail "ipc socket never appeared"; }
sleep 2   # let the theme style + pane shells settle

# --- window geometry (absolute screen coords; do NOT assume 0,0) ---------
step "locate window"
WID=""
for _ in $(seq 1 20); do
    WID=$(xdotool search --onlyvisible --name terminator-rust 2>/dev/null | head -1 || true)
    [ -n "$WID" ] && break
    sleep 0.25
done
[ -n "$WID" ] || fail "app window not found"
xdotool windowfocus "$WID"   # no WM: give the window X focus
eval "$(xdotool getwindowgeometry --shell "$WID")"   # -> X Y WIDTH HEIGHT
export X Y WIDTH HEIGHT   # for the PIL checkers below
echo "window $WID at ${X},${Y} ${WIDTH}x${HEIGHT}"
kill -0 "$APP_PID" 2>/dev/null || fail "app died during startup"

step "sanity: preset loaded (3 panes) + id resolution"
[ "$("$CTL" list --json | grep -c '"id": ')" = "3" ] \
    || { "$CTL" list --json; fail "expected 3 panes (alpha split + beta)"; }
# Restore remaps pane ids (seed_alloc): resolve the REAL id by title.
# NB: the json arrives via an env var, NOT stdin - `python3 -` takes its
# program from the heredoc, so stdin is already consumed.
IDS=$(LIST_JSON="$("$CTL" list --json)" python3 - <<'PY'
import json, os, sys
panes = json.loads(os.environ["LIST_JSON"])
panes = panes["panes"] if isinstance(panes, dict) else panes
beta = next((p["id"] for p in panes if p.get("name") == "notes"), None)
if beta is None:
    sys.exit("pane lookup failed: no pane titled 'notes'")
print(f"BETA_ID={beta}")
PY
) || fail "could not resolve pane ids from list --json"
eval "$IDS"
echo "BETA_ID=$BETA_ID (pane 'notes': plain local pane, later keeper target)"

# --- helpers --------------------------------------------------------------

# shot <png>: park the pointer on bare chrome (Xvfb rests it at the
# screen center = a split gutter; hover fills must not skew the scan),
# let an app frame hit-test it, then scrot.
shot() {
    xdotool mousemove $((X + WIDTH * 30 / 100)) $((Y + 6))
    sleep 0.5
    scrot -o "$1"
}

# dot_scan <png> <expect|forbid>: scan the chip row band (Y+6..Y+30) for
# badge-blue pixels (70,150,255 +-12/channel), cluster matching x coords
# by gaps > 4px, print "count=<n> clusters=<k> xmin=<a> xmax=<b>" and set
# BLUE_MIN/BLUE_MAX shell globals. expect: count >= 30 AND clusters == 1
# (one solid dot, no second badge); forbid: count == 0.
dot_scan() {
    local out
    out=$(CAP=$1 DOTMODE=$2 python3 - <<'PY'
import os, sys
from PIL import Image
X, Y = int(os.environ["X"]), int(os.environ["Y"])
img = Image.open(os.environ["CAP"]).convert("RGB")
BLUE, TOL = (70, 150, 255), 12
pts = [(x, yy) for yy in range(Y + 6, Y + 30)
       for x in range(X, X + min(int(os.environ["WIDTH"]), 500))
       if all(abs(a - b) <= TOL for a, b in zip(img.getpixel((x, yy)), BLUE))]
xs = sorted({x for x, _ in pts})     # dedup columns for the clustering
clusters, prev = 0, None
for x in xs:
    if prev is None or x - prev > 4:
        clusters += 1
    prev = x
count = len(pts)                     # matched PIXELS (a 5px dot is ~70)
xmin = xs[0] if xs else 0
xmax = xs[-1] if xs else 0
if os.environ["DOTMODE"] == "expect" and (count < 30 or clusters != 1):
    sys.exit(f"expected ONE badge cluster, got count={count} "
             f"clusters={clusters} x {xmin}..{xmax}")
if os.environ["DOTMODE"] != "expect" and count != 0:
    sys.exit(f"forbidden badge pixels: count={count} x {xmin}..{xmax}")
print(f"count={count} clusters={clusters} xmin={xmin} xmax={xmax}")
print(f"BLUE_MIN={xmin} BLUE_MAX={xmax}")
PY
    ) || fail "dot_scan $2 failed for $1"
    echo "$out" | head -1
    eval "$out"
}

# wait_pane <pane> <marker> [tries]: poll `ctl capture` until the pane's
# visible screen contains the literal marker.
wait_pane() {
    local pane=$1 marker=$2 tries=${3:-40}
    for _ in $(seq 1 "$tries"); do
        if "$CTL" capture "$pane" 2>/dev/null | grep -qF -- "$marker"; then
            return 0
        fi
        sleep 0.25
    done
    return 1
}

# cap_forbid <capture-file> <literal> <why>: a pane capture must NOT
# contain the literal (error text / rendered OSC garbage).
cap_forbid() {
    if grep -qF -- "$2" "$1"; then fail "$3 (capture shows '$2')"; fi
}

# wait_active <0|1>: poll state.json until the persisted active_tab
# matches (tab switches must dirty the state file).
wait_active() {
    local i=0
    for _ in $(seq 1 20); do
        [ "$(active_tab)" = "$1" ] && { i=1; break; }
        sleep 0.25
    done
    [ "$i" = "1" ] || fail "active_tab never became $1"
}

# click_alpha: click the alpha chip (LEFT of the active beta chip):
# the chips sit <=8px apart, so BETA_L-30 stays >15px clear of alpha's
# 14px close strip and well inside its body.
click_alpha() {
    local out
    shot "$ROOT/click.png"
    out=$(CHIPSCAN="$ROOT/click.png" e2e_chip_edges) || fail "active-chip scan failed"
    click_at $(( ${out% *} - 30 )) $((Y + 18))
    wait_active 0
}

# click_beta_badged: click the beta chip just LEFT of its badge dot
# (BLUE_MIN-12 sits ~26px clear of the 14px close strip, in the body).
click_beta_badged() {
    click_at $((BLUE_MIN - 12)) $((Y + 18))
    wait_active 1
}

step "setup: pane 13 can resolve terminator-ctl on PATH"
# Typed hooks must work by bare name (real usage); the same export is
# inherited by the later `terminator-session attach` -> keeper shell.
"$CTL" send "$BETA_ID" --text "export PATH=\"$BINDIR:\$PATH\""$'\n' >/dev/null
wait_pane "$BETA_ID" "export PATH" \
    || { "$CTL" capture "$BETA_ID"; fail "pane $BETA_ID never echoed the PATH export"; }
sleep 0.8

# --- N1: notice typed into a PLAIN pane on a background tab ---------------
step "N1: 'terminator-ctl notice' in plain pane 13 -> badge right of the active chip"
"$CTL" send "$BETA_ID" --text "terminator-ctl notice"$'\n' >/dev/null
sleep 1.5
"$CTL" capture "$BETA_ID" >"$ROOT/n1.cap" || fail "capture of the plain pane failed"
# Success is SILENT: the only trace of the command is its own echo, and
# the OSC 9 payload must be consumed by the VT, not rendered as text.
[ "$(grep -c "terminator-ctl notice" "$ROOT/n1.cap")" = "1" ] \
    || { cat "$ROOT/n1.cap"; fail "notice printed output (command echoed more than once)"; }
cap_forbid "$ROOT/n1.cap" "]9;" "OSC 9 payload rendered as text in pane 13"
cap_forbid "$ROOT/n1.cap" "error:" "ctl error text rendered in pane 13"
shot "$ROOT/n1.png"
out=$(CHIPSCAN="$ROOT/n1.png" e2e_chip_edges) || fail "active-chip scan failed (N1)"
ACT_R=${out#* }                     # active (alpha) chip right edge
dot_scan "$ROOT/n1.png" expect
[ "$BLUE_MIN" -gt $((ACT_R + 5)) ] \
    || fail "badge ($BLUE_MIN) not right of the active chip (right $ACT_R)"
N1_MIN=$BLUE_MIN
echo "PASS N1: badge at $BLUE_MIN..$BLUE_MAX, active chip right $ACT_R, silent success"

# --- N2: repeated notices dedup into one badge ----------------------------
step "N2: notice again -> still one cluster"
"$CTL" send "$BETA_ID" --text "terminator-ctl notice"$'\n' >/dev/null
sleep 1.5
shot "$ROOT/n2.png"
dot_scan "$ROOT/n2.png" expect
echo "PASS N2: deduped to a single badge cluster"

# --- N3: negative - any argument is a usage error -------------------------
step "N3: 'terminator-ctl notice foo' -> usage error rc!=0, badge unchanged"
set +e
"$CTL" notice foo >"$ROOT/n3.out" 2>"$ROOT/n3.err"
RC=$?
set -e
[ "$RC" -ne 0 ] || fail "'notice foo' exited 0 - any argument must be a usage error"
[ ! -s "$ROOT/n3.out" ] || { cat "$ROOT/n3.out"; fail "'notice foo' wrote stdout"; }
grep -qF "notice takes no arguments" "$ROOT/n3.err" \
    || { cat "$ROOT/n3.err"; fail "usage reason missing from stderr"; }
grep -q "notice" "$ROOT/n3.err" || fail "usage text missing the notice line"
# The bare command must ALWAYS exit 0 (best-effort contract): stdout is
# redirected so the raw bytes never pollute this log.
if ! "$CTL" notice >/dev/null 2>"$ROOT/n3b.err"; then
    fail "bare 'terminator-ctl notice' exited non-zero (must always be 0)"
fi
shot "$ROOT/n3.png"
dot_scan "$ROOT/n3.png" expect      # pane 13's badge unchanged: still ONE
echo "PASS N3: rc=$RC + usage error text, bare command rc=0, badge unchanged"

# --- N4: click the badged chip -> tab activates, badge acknowledged -------
step "N4: click the badged chip -> active_tab=1, badge cleared"
shot "$ROOT/n4.png"
dot_scan "$ROOT/n4.png" expect      # fresh BLUE_MIN right before the click
click_beta_badged
shot "$ROOT/n4b.png"
dot_scan "$ROOT/n4b.png" forbid
echo "PASS N4: focused window acknowledged its active tab"

# --- N5: notice while the tab is focused+active -> suppressed -------------
step "N5: notice from pane 13 while beta IS the focused active tab"
"$CTL" send "$BETA_ID" --text "terminator-ctl notice"$'\n' >/dev/null
sleep 1.5
shot "$ROOT/n5.png"
dot_scan "$ROOT/n5.png" forbid
echo "PASS N5: active-tab notice suppressed/acked - zero clusters"

# --- N6: background beta again -> notice re-badges at the same x ----------
step "N6: switch back to alpha, notice again -> badge reappears"
click_alpha
"$CTL" send "$BETA_ID" --text "terminator-ctl notice"$'\n' >/dev/null
sleep 1.5
shot "$ROOT/n6.png"
out=$(CHIPSCAN="$ROOT/n6.png" e2e_chip_edges) || fail "active-chip scan failed (N6)"
ACT_R=${out#* }                      # active (alpha) chip right edge
dot_scan "$ROOT/n6.png" expect
[ "$BLUE_MIN" -gt $((ACT_R + 5)) ] \
    || fail "badge ($BLUE_MIN) not right of the active chip (right $ACT_R)"
[ $((BLUE_MIN - N1_MIN)) -le 3 ] && [ $((N1_MIN - BLUE_MIN)) -le 3 ] \
    || fail "badge moved (N1 x $N1_MIN vs N6 x $BLUE_MIN)"
echo "PASS N6: badge back at $BLUE_MIN..$BLUE_MAX (N1 $N1_MIN), right of the active chip (right $ACT_R)"

# --- N7: zero-binary fallback - bare printf of the same bytes -------------
step "N7: raw printf fallback -> badge at the same x, ack clears"
click_beta_badged                    # clear N6's badge first
shot "$ROOT/n7a.png"
dot_scan "$ROOT/n7a.png" forbid
click_alpha                          # beta must be a background tab again
# DOUBLE backslash on purpose: ctl send --text carries \033 as TEXT so
# bash's own printf inside the pane interprets it (single \033 sends a
# literal ESC byte and readline mangles the line - see agents.md).
"$CTL" send "$BETA_ID" --text "printf '\\033]9;terminator-rust notice\\007'"$'\n' >/dev/null
sleep 1.5
"$CTL" capture "$BETA_ID" >"$ROOT/n7.cap" || fail "capture of the plain pane failed"
[ "$(grep -c "terminator-rust notice" "$ROOT/n7.cap")" = "1" ] \
    || { cat "$ROOT/n7.cap"; fail "printf echo not visible exactly once"; }
shot "$ROOT/n7b.png"
out=$(CHIPSCAN="$ROOT/n7b.png" e2e_chip_edges) || fail "active-chip scan failed (N7)"
ACT_R=${out#* }                      # active (alpha) chip right edge
dot_scan "$ROOT/n7b.png" expect
[ "$BLUE_MIN" -gt $((ACT_R + 5)) ] \
    || fail "badge ($BLUE_MIN) not right of the active chip (right $ACT_R)"
[ $((BLUE_MIN - N1_MIN)) -le 3 ] && [ $((N1_MIN - BLUE_MIN)) -le 3 ] \
    || fail "printf badge moved (N1 x $N1_MIN vs N7 x $BLUE_MIN)"
N7_MIN=$BLUE_MIN; N7_MAX=$BLUE_MAX
click_beta_badged
shot "$ROOT/n7c.png"
dot_scan "$ROOT/n7c.png" forbid
echo "PASS N7: printf badge at $N7_MIN..$N7_MAX (N1 $N1_MIN), acked"

# --- N8: keeper traversal - notice INSIDE a terminator-session attach -----
step "N8: attach pane 13 to keeper 'notes', notice inside -> badge"
click_alpha                          # beta stays a background tab
shot "$ROOT/n8a.png"
dot_scan "$ROOT/n8a.png" forbid      # clean slate before the keeper case
# attach hands the pane over: pane 13's tty goes raw and every keystroke
# now feeds the keeper's private pty (PATH export was inherited).
"$CTL" send "$BETA_ID" --text "$BINDIR/terminator-session attach notes --title notes"$'\n' >/dev/null
wait_pane "$BETA_ID" "attach notes --title notes" \
    || { "$CTL" capture "$BETA_ID"; fail "pane $BETA_ID never echoed the attach command"; }
KSOCK="$HOME/.terminator-rust/sessions/notes.sock"
KUP=""
for _ in $(seq 1 40); do
    [ -S "$KSOCK" ] && { KUP=1; break; }
    sleep 0.25
done
[ -n "$KUP" ] || fail "keeper socket $KSOCK never appeared"
# setsid-detached keeper: remember its pids so the EXIT trap never
# leaks the daemon (pgrep -f never matches this script's own argv).
KEEPER_PIDS=$(pgrep -f "terminator-session __serve notes" || true)
[ -n "$KEEPER_PIDS" ] || fail "keeper daemon pid not found"
ATTACHED=""
for _ in $(seq 1 40); do
    [ "$("$TS" list | awk '$1=="notes" {print $2}')" = "attached" ] \
        && { ATTACHED=1; break; }
        sleep 0.25
done
[ -n "$ATTACHED" ] || { "$TS" list; fail "keeper never reported 'notes' as attached"; }
sleep 1.5   # keeper shell first prompt
# The hook runs in the keeper shell: ctl writes the OSC 9 bytes to ITS
# /dev/tty = the keeper's pty; the attach stream carries them back to
# pane 13 as ordinary output, so the VT there fires the badge.

"$CTL" send "$BETA_ID" --text "terminator-ctl notice"$'\n' >/dev/null
sleep 1.5
"$CTL" capture "$BETA_ID" >"$ROOT/n8.cap" || fail "capture of the keeper pane failed"
# The pane keeps its pre-attach scrollback, so do not COUNT echoes:
# prove the notice ran INSIDE the keeper shell (echo below the attach
# echo) and that success stayed silent.
ATT_LINE=$(grep -n "attach notes --title notes" "$ROOT/n8.cap" | tail -1 | cut -d: -f1)
NOTICE_LINE=$(grep -n "terminator-ctl notice" "$ROOT/n8.cap" | tail -1 | cut -d: -f1)
[ -n "$ATT_LINE" ] && [ -n "$NOTICE_LINE" ] && [ "$NOTICE_LINE" -gt "$ATT_LINE" ] \
    || { cat "$ROOT/n8.cap"; fail "notice echo not below the attach echo (keeper shell never ran it)"; }
# Forbids apply to the KEEPER-shell region only (the pre-attach
# scrollback legitimately contains N7's printf echo text).
tail -n +"$((ATT_LINE + 1))" "$ROOT/n8.cap" >"$ROOT/n8.keep.cap"
cap_forbid "$ROOT/n8.keep.cap" "]9;" "OSC 9 payload rendered as text in the keeper pane"
cap_forbid "$ROOT/n8.keep.cap" "takes no arguments" "usage error rendered in the keeper pane"
shot "$ROOT/n8.png"
out=$(CHIPSCAN="$ROOT/n8.png" e2e_chip_edges) || fail "active-chip scan failed (N8)"
ACT_R=${out#* }                      # active (alpha) chip right edge
dot_scan "$ROOT/n8.png" expect
[ "$BLUE_MIN" -gt $((ACT_R + 5)) ] \
    || fail "badge ($BLUE_MIN) not right of the active chip (right $ACT_R)"
N8_MIN=$BLUE_MIN; N8_MAX=$BLUE_MAX
click_beta_badged
shot "$ROOT/n8b.png"
dot_scan "$ROOT/n8b.png" forbid
echo "PASS N8: keeper attach traversal badged at $N8_MIN..$N8_MAX, acked"

# --- N9: liveness ----------------------------------------------------------
step "N9: app + keeper still answer"
kill -0 "$APP_PID" 2>/dev/null || fail "app process died during the notice checks"
"$CTL" list --json >/dev/null || fail "ctl list stopped answering"
[ "$("$TS" list | awk '$1=="notes" {print $2}')" = "attached" ] \
    || { "$TS" list; fail "keeper session 'notes' lost"; }
echo "PASS N9"

echo "e2e-notice: ALL PASS"
