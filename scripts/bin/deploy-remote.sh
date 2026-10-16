#!/usr/bin/env bash
# One-click remote deploy for terminator-rust: build + pack, verify the
# sha256 gate locally, upload + install under /opt on the target host,
# install an XDG autostart entry for the desktop user, restart the live
# app over ssh, and smoke it through the control socket via terminator-ctl.
#
# Stages (each announced with step()):
#   1. build+pack  release build, fmt/clippy/tests, then pack-release.sh.
#                  Skipped with --no-build (dist/ must already exist).
#   2. verify      check SHA256 and run real IME tests on the archive's binaries.
#   3. upload      verify and install into releases/SHA256; journal the old
#                  artifact before atomically switching current.
#   4. autostart   idempotent ~/.config/autostart/terminator-rust.desktop
#                  for the desktop user (Exec=/Icon= via the `current`
#                  symlink).
#   5. restart     stop any running instance (kill by pid, exe-checked),
#                  start fresh on the desktop DISPLAY, wait for the
#                  control socket.
#   6. smoke       terminator-ctl list + capture round-trip, PASS summary.
#
# Usage: scripts/bin/deploy-remote.sh [--no-build] [--host HOST]
#                                      [--user USER] [--root ROOTUSER]
# Defaults: HOST=192.168.31.196 USER=m ROOTUSER=root.
# All ssh is key-based (BatchMode). Remote scripts run under `bash -s`
# regardless of the login shell (the desktop user uses zsh; never rely on
# bare glob expansion there). Never pkill -f a self-matching pattern:
# candidate pids are filtered through /proc/<pid>/exe before any kill.
set -euo pipefail
cd "$(dirname "$0")/../.."

fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo "== $*"; }

usage() {
    cat <<'USAGE'
Usage: scripts/bin/deploy-remote.sh [OPTIONS]

One-click deploy of terminator-rust to a remote host.

Options:
  --no-build        skip cargo build + dist/ repack (artifacts must exist)
  --keep-running    do not stop a running instance: its windows stay untouched,
                    the new build goes live when that app next launches
  --rollback SHA    restore the previous release recorded for this artifact
  --host HOST       target host            (default: 192.168.31.196)
  --user USER       desktop/app user       (default: m)
  --root ROOTUSER   ssh user for /opt ops  (default: root)
  -h, --help        show this help
USAGE
}

# --- CLI -------------------------------------------------------------------
HOST=192.168.31.196
DEPLOY_USER=m
ROOT_USER=root
DO_BUILD=1
ROLLBACK=""
KEEP_RUNNING=0      # --keep-running: leave the live instance (and windows) alone
KEPT=0              # set by stage_restart when it left an instance running
while [ $# -gt 0 ]; do
    case "$1" in
        --no-build) DO_BUILD=0 ;;
        --keep-running) KEEP_RUNNING=1 ;;
        --rollback)
            [ $# -ge 2 ] || fail "--rollback requires an artifact SHA256"
            ROLLBACK=$2; shift ;;
        --host|--user|--root)
            [ $# -ge 2 ] || fail "$1 needs a value"
            case "$1" in
                --host) HOST=$2 ;;
                --user) DEPLOY_USER=$2 ;;
                --root) ROOT_USER=$2 ;;
            esac
            shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
    shift
done
if [ -n "$ROLLBACK" ]; then
    [[ "$ROLLBACK" =~ ^[0-9a-f]{64}$ ]] || fail "--rollback requires a full lowercase SHA256"
fi

# --- deploy layout (facts of the target, not CLI knobs) --------------------
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10)
OPT_ROOT=/opt/terminator-rust          # root-owned install root
DESKTOP_DISPLAY=:0                     # LightDM X session of the app user
RUNTIME_DIR=/run/user/1000             # XDG_RUNTIME_DIR of the app user
LOG_FILE=/tmp/terminator-rust.log      # app stdout/stderr on the target

VERSION=$(sed -n 's/^version *= *"\([^"]*\)" *$/\1/p' Cargo.toml | head -n1)
[ -n "$VERSION" ] || fail "cannot read workspace version from Cargo.toml"
DIST=dist
TRIPLE=x86_64-unknown-linux-gnu
if [ -z "$ROLLBACK" ]; then
    [ "$(rustc -vV | sed -n 's/^host: //p')" = "$TRIPLE" ] \
        || fail "remote deployment requires a Linux x86_64 build host"
fi
DIRNAME="terminator-rust-$VERSION-$TRIPLE"
ARTIFACT=""
TGZ="$DIST/$DIRNAME.tar.gz"

# run_user / run_root: run a bash script (stdin) on the host as the app
# user / root user; script args come after --, one shell word each (ssh
# joins them with spaces, so no embedded whitespace in args).
run_user() { ssh "${SSH_OPTS[@]}" "$DEPLOY_USER@$HOST" bash -s -- "$@"; }
run_root() { ssh "${SSH_OPTS[@]}" "$ROOT_USER@$HOST" bash -s -- "$@"; }

# --- stage 1: build + pack -------------------------------------------------
stage_build() {
    if [ "$DO_BUILD" -eq 0 ]; then
        step "1/6 build+pack: skipped (--no-build)"
        [ -f "$TGZ" ] && [ -f "$TGZ.sha256" ] \
            || fail "--no-build but '$TGZ' or '$TGZ.sha256' is missing; rerun without --no-build"
        return
    fi
    step "1/6 build + pack $DIRNAME"
    [ -f README.md ] || fail "README.md missing at repo root"
    cargo build --release --workspace --target-dir "$PWD/target"
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --target-dir "$PWD/target" -- -D warnings
    cargo test --workspace --target-dir "$PWD/target"
    python3 scripts/tests/deploy/test_release.py
    scripts/bin/pack-release.sh "$TRIPLE"
}

# --- stage 2: local sha gate ------------------------------------------------
stage_verify() {
    step "2/6 verify the release archive"
    (cd "$DIST" && sha256sum -c "$DIRNAME.tar.gz.sha256") || fail "archive checksum mismatch"
    ARTIFACT=$(sha256sum "$TGZ" | cut -d ' ' -f1)
    bash scripts/deploy/verify-release.sh "$TGZ" || fail "candidate archive failed release checks"
    [ "$(sha256sum "$TGZ" | cut -d ' ' -f1)" = "$ARTIFACT" ] || fail "archive changed after verification"
}

# The remote helper checks its own archive hash, installs to releases/SHA256,
# then journals the previous target before atomically switching current.
release_action() {
    ssh "${SSH_OPTS[@]}" "$ROOT_USER@$HOST" python3 - "$OPT_ROOT" "$@" \
        < scripts/deploy/release.py
}

stage_upload() {
    step "3/6 install immutable artifact $ARTIFACT"
    release_action preflight || fail "release identity cannot write the install root"
    scp "${SSH_OPTS[@]}" "$TGZ" "$ROOT_USER@$HOST:$OPT_ROOT/incoming/$ARTIFACT.tar.gz" \
        || fail "archive upload failed"
    release_action install "$ARTIFACT" || fail "release installation failed"
    echo "rollback: scripts/bin/deploy-remote.sh --host $HOST --user $DEPLOY_USER --root $ROOT_USER --rollback $ARTIFACT"
}

# --- stage 4: XDG autostart entry (idempotent) -------------------------------
stage_autostart() {
    step "4/6 install autostart entry for $DEPLOY_USER"
    run_user "$OPT_ROOT" <<'REMOTE' || fail "autostart install failed"
set -e
root=$1
d=$HOME/.config/autostart
mkdir -p "$d"
cat > "$d/terminator-rust.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=Terminator Rust
Comment=terminator-rust terminal multiplexer
Exec=$root/current/bin/terminator-rust
Icon=$root/current/share/icons/hicolor/256x256/apps/terminator-rust.png
Terminal=false
X-GNOME-Autostart-enabled=true
DESK
chmod 0644 "$d/terminator-rust.desktop"
grep -Fx "Exec=$root/current/bin/terminator-rust" "$d/terminator-rust.desktop"
icon=$root/current/share/icons/hicolor/256x256/apps/terminator-rust.png
grep -Fx "Icon=$icon" "$d/terminator-rust.desktop"
test -f "$icon"
stat -c '%a %U %n' "$d/terminator-rust.desktop"
REMOTE
}

# --- stage 5: restart the app ------------------------------------------------
stage_restart() {
    step "5/6 restart app as $DEPLOY_USER on $HOST"
    # Candidates by cmdline AND by process name: an instance started from
    # a desktop/panel launcher carries a bare "terminator-rust" argv[0]
    # (no install path), so the -f pattern alone misses it - that is how a
    # deploy once left the old instance running beside the fresh one.
    # /proc/<pid>/exe is still the only gate that decides: a candidate is
    # kept only when the running image lives under $root (never kill a
    # foreign process; the ssh/zsh wrapper carrying this script text
    # resolves elsewhere and is dropped).
    local stop_out kept_pids
    if ! stop_out=$(run_user "$OPT_ROOT" "$RUNTIME_DIR" "$KEEP_RUNNING" <<'REMOTE'
root=$1; rt=$2; keep=${3:-0}
pids_of() {
    local cand p
    cand=$( (
        pgrep -f "$root/.*/bin/terminator-rust" || true
        pgrep -x terminator-rust || true
    ) | sort -u )
    for p in $cand; do
        # a pgrep hit can surface before execve lands -> empty readlink
        # (same race e2e-oc-exit.sh K1 guards). Retry only the EMPTY read,
        # bounded 20x0.25s: wrapper shells carrying this script text never
        # match $root/*, so break-to-match would burn 5s per wrapper on
        # every pids_of call (the wait loop calls it ~40x).
        exe=""
        for _ in $(seq 1 20); do
            exe=$(readlink "/proc/$p/exe" 2>/dev/null || true)
            [ -n "$exe" ] && break
            sleep 0.25
        done
        # Older deployments could replace binaries in place. Keep detecting
        # those existing processes while new artifacts stay immutable.
        exe=${exe% (deleted)}
        case "$exe" in "$root"/*) echo "$p" ;; esac
    done
}
old=$(pids_of)
if [ -n "$old" ] && [ "$keep" = "1" ]; then
    # --keep-running: leave the live instance (and its windows) alone; do
    # NOT remove the control socket below or the running app loses IPC.
    echo "DEPLOY-KEEP:$old"
    exit 0
fi
if [ -n "$old" ]; then
    for p in $old; do echo "stopping pid $p"; kill "$p" 2>/dev/null || true; done
    for _ in $(seq 1 40); do
        [ -z "$(pids_of)" ] && break
        sleep 0.25
    done
    for p in $(pids_of); do
        echo "pid $p did not exit in 10s, SIGKILL" >&2
        kill -9 "$p" 2>/dev/null || true
    done
fi
# A stale socket file would fake the wait below; the app reclaims it on
# start anyway, so drop it now.
rm -f "$rt/terminator-rust/ipc.sock"
REMOTE
    ); then
        fail "stopping the old instance failed"
    fi
    printf '%s\n' "$stop_out"
    kept_pids=$(printf '%s\n' "$stop_out" | sed -n 's/^DEPLOY-KEEP:\(.*\)$/\1/p')
    if [ -n "$kept_pids" ]; then
        echo "keeping running instance(s) $kept_pids - windows untouched"
        KEPT=1
        return
    fi
    run_user "$OPT_ROOT" "$DESKTOP_DISPLAY" "$RUNTIME_DIR" "$LOG_FILE" <<'REMOTE' \
        || fail "app launch failed"
root=$1; disp=$2; rt=$3; log=$4
# winit XIM only connects when XMODIFIERS points at a live IM daemon.
# The ssh launch path misses the desktop session env (autostart-launched
# instances inherit it), so probe the desktop user's daemon and mirror it.
im=""
me=$(id -un)
if pgrep -u "$me" -x fcitx5 >/dev/null 2>&1 \
    || pgrep -u "$me" -x fcitx >/dev/null 2>&1; then
    im="@im=fcitx"
elif pgrep -u "$me" -x ibus-daemon >/dev/null 2>&1; then
    im="@im=ibus"
fi
[ -n "$im" ] && export XMODIFIERS="$im"
DISPLAY="$disp" XDG_RUNTIME_DIR="$rt" \
    setsid nohup "$root/current/bin/terminator-rust" \
    >> "$log" 2>&1 </dev/null &
echo "launched on DISPLAY=$disp, log $log${im:+, XMODIFIERS=$im}"
REMOTE
    local i up=0
    for i in $(seq 1 30); do
        if run_user "$RUNTIME_DIR" <<'REMOTE'
[ -S "$1/terminator-rust/ipc.sock" ]
REMOTE
        then up=1; break; fi
        sleep 0.5
    done
    if [ "$up" -ne 1 ]; then
        run_user "$RUNTIME_DIR" "$LOG_FILE" <<'REMOTE' >&2
rt=$1; log=$2
ls -la "$rt/terminator-rust" 2>&1 || true
tail -n 20 "$log" 2>&1 || true
REMOTE
        fail "control socket never appeared after 15s"
    fi
}

# --- stage 6: smoke over the control socket ----------------------------------
stage_smoke() {
    step "6/6 smoke: terminator-ctl list + capture"
    local list_out pane_id cap
    # A KEPT instance may listen on a non-default socket name (the .196 box
    # ran migrate3.sock until the 2026-10-17 live cutover): every snippet below
    # prefers the running app's own TERMINATOR_SOCK (from /proc/<pid>/environ)
    # so the smoke reaches the live app instead of a stale ipc.sock - the
    # kept instance is exactly the case --keep-running promises to probe.
    list_out=$(run_user "$OPT_ROOT" "$RUNTIME_DIR" "$KEPT" <<'REMOTE'
root=$1; rt=$2; keep=${3:-0}
export XDG_RUNTIME_DIR="$rt"
if [ "$keep" = "1" ]; then
    for p in $(pgrep -x terminator-rust); do
        s=$(tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | sed -n 's/^TERMINATOR_SOCK=//p' | head -n1)
        [ -n "$s" ] && { export TERMINATOR_SOCK="$s"; break; }
    done
fi
ctl="$root/current/bin/terminator-ctl"
for _ in $(seq 1 40); do
    out=$("$ctl" list 2>/dev/null || true)
    if printf '%s\n' "$out" | sed -n '2p' | grep -q '^[0-9]'; then
        printf '%s\n' "$out"
        exit 0
    fi
    sleep 0.25
done
echo "no pane row in 'ctl list' after 10s" >&2
exit 1
REMOTE
    ) || fail "terminator-ctl list failed (see output above)"
    printf '%s\n' "$list_out"
    pane_id=$(run_user "$OPT_ROOT" "$RUNTIME_DIR" "$KEPT" <<'REMOTE'
root=$1; rt=$2; keep=${3:-0}
export XDG_RUNTIME_DIR="$rt"
if [ "$keep" = "1" ]; then
    for p in $(pgrep -x terminator-rust); do
        s=$(tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | sed -n 's/^TERMINATOR_SOCK=//p' | head -n1)
        [ -n "$s" ] && { export TERMINATOR_SOCK="$s"; break; }
    done
fi
"$root/current/bin/terminator-ctl" list --json \
    | grep -o '"id": *[0-9]\+' | head -n1 | grep -o '[0-9]\+'
REMOTE
    ) || fail "could not extract a pane id from 'list --json'"
    [ -n "$pane_id" ] || fail "empty pane id from 'list --json'"
    echo "first pane id: $pane_id"
    cap=$(run_user "$OPT_ROOT" "$RUNTIME_DIR" "$pane_id" "$KEPT" <<'REMOTE'
root=$1; rt=$2; pane=$3; keep=${4:-0}
export XDG_RUNTIME_DIR="$rt"
if [ "$keep" = "1" ]; then
    for p in $(pgrep -x terminator-rust); do
        s=$(tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | sed -n 's/^TERMINATOR_SOCK=//p' | head -n1)
        [ -n "$s" ] && { export TERMINATOR_SOCK="$s"; break; }
    done
fi
for _ in $(seq 1 40); do
    out=$("$root/current/bin/terminator-ctl" capture "$pane" 2>/dev/null || true)
    if [ -n "$(printf '%s' "$out" | tr -d '[:space:]')" ]; then
        printf '%s\n' "$out"
        exit 0
    fi
    sleep 0.25
done
echo "capture of pane $pane still empty after 10s" >&2
exit 1
REMOTE
    ) || fail "terminator-ctl capture failed"
    echo "--- capture pane $pane_id ---"
    printf '%s\n' "$cap"
    if [ "$KEPT" -eq 1 ]; then
        echo "note: smoke ran against the previously-running instance; the new build goes live at its next launch"
    fi
    if [ "$KEPT" -eq 0 ]; then
        run_user "$OPT_ROOT" <<'REMOTE' || fail "running binary differs from selected artifact"
root=$1
wanted=$(readlink -f "$root/current/bin/terminator-rust")
for pid in $(pgrep -x terminator-rust); do
    [ "$(readlink "/proc/$pid/exe")" = "$wanted" ] && { echo "running artifact: $wanted"; exit 0; }
done
exit 1
REMOTE
    fi
    echo "PASS: $HOST list+capture OK (kept-running=$KEPT)"
}

# --- main --------------------------------------------------------------------
step "deploy terminator-rust $VERSION -> $DEPLOY_USER@$HOST ($OPT_ROOT)"
run_root </dev/null || fail "cannot ssh $ROOT_USER@$HOST (key auth?)"
run_user </dev/null || fail "cannot ssh $DEPLOY_USER@$HOST (key auth?)"
if [ -n "$ROLLBACK" ]; then
    release_action rollback "$ROLLBACK" || fail "rollback refused"
else
    stage_build
    stage_verify
    stage_upload
fi
stage_autostart
stage_restart
stage_smoke
step "done"
