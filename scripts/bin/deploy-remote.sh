#!/usr/bin/env bash
# One-click remote deploy for terminator-rust: build + pack, verify the
# sha256 gate locally, upload + install under /opt on the target host,
# install an XDG autostart entry for the desktop user, restart the live
# app over ssh, and smoke it through the control socket via terminator-ctl.
#
# Stages (each announced with step()):
#   1. build+pack  cargo build --release, then a deterministic repack of
#                  dist/ (staging dir + tar.gz + SHA256SUMS.txt).
#                  Skipped with --no-build (dist/ must already exist).
#   2. verify      sha256sum -c SHA256SUMS.txt locally in dist/.
#   3. upload      scp tarball + sums to root; remote sha256sum -c gate;
#                  drop the old extracted dir, re-extract, repoint the
#                  stable symlink /opt/terminator-rust/current.
#   4. autostart   idempotent ~/.config/autostart/terminator-rust.desktop
#                  for the desktop user (Exec= via the `current` symlink).
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
while [ $# -gt 0 ]; do
    case "$1" in
        --no-build) DO_BUILD=0 ;;
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

# --- deploy layout (facts of the target, not CLI knobs) --------------------
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10)
OPT_ROOT=/opt/terminator-rust          # root-owned install root
DESKTOP_DISPLAY=:0                     # LightDM X session of the app user
RUNTIME_DIR=/run/user/1000             # XDG_RUNTIME_DIR of the app user
LOG_FILE=/tmp/terminator-rust.log      # app stdout/stderr on the target

VERSION=$(sed -n 's/^version *= *"\([^"]*\)" *$/\1/p' Cargo.toml | head -n1)
[ -n "$VERSION" ] || fail "cannot read workspace version from Cargo.toml"
DIST=dist
DIRNAME="terminator-rust-$VERSION-linux-x86_64"
STAGE="$DIST/$DIRNAME"
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
        [ -f "$TGZ" ] && [ -f "$DIST/SHA256SUMS.txt" ] \
            || fail "--no-build but '$TGZ' or '$DIST/SHA256SUMS.txt' is missing; rerun without --no-build"
        return
    fi
    step "1/6 build + pack $DIRNAME"
    [ -f README.md ] || fail "README.md missing at repo root"
    cargo build --release
    rm -rf "$STAGE"
    mkdir -p "$STAGE/bin"
    install -m 0755 target/release/terminator-rust "$STAGE/bin/terminator-rust"
    install -m 0755 target/release/terminator-ctl "$STAGE/bin/terminator-ctl"
    install -m 0755 target/release/terminator-rust "$STAGE/terminator-rust"
    install -m 0755 target/release/terminator-ctl "$STAGE/terminator-ctl"
    install -m 0644 README.md "$STAGE/README.md"
    # Deterministic archive: sorted names, zeroed mtime/owner, gzip -n
    # (no timestamp) -> identical tarball for identical inputs.
    tar -C "$DIST" --sort=name --mtime='UTC 1970-01-01' --owner=0 \
        --group=0 --numeric-owner -c "$DIRNAME" | gzip -n > "$TGZ"
    (cd "$DIST" && sha256sum "$DIRNAME.tar.gz" > SHA256SUMS.txt)
}

# --- stage 2: local sha gate ------------------------------------------------
stage_verify() {
    step "2/6 verify dist/ against SHA256SUMS.txt"
    (cd "$DIST" && sha256sum -c SHA256SUMS.txt) \
        || fail "local sha256sum -c failed; rerun without --no-build to repack"
}

# --- stage 3: upload + install under /opt -----------------------------------
stage_upload() {
    step "3/6 upload + install under $OPT_ROOT on $HOST"
    scp "${SSH_OPTS[@]}" "$TGZ" "$DIST/SHA256SUMS.txt" \
        "$ROOT_USER@$HOST:$OPT_ROOT/" \
        || fail "scp to $ROOT_USER@$HOST:$OPT_ROOT failed"
    run_root "$OPT_ROOT" "$DIRNAME" <<'REMOTE' || fail "remote install failed"
set -e
root=$1; name=$2
cd "$root"
sha256sum -c SHA256SUMS.txt          # gate: wire bits match local dist/
rm -rf "$root/$name"                 # old extracted dir of this version
tar -xzf "$root/$name.tar.gz" -C "$root"
ln -sfn "$root/$name" "$root/current" # stable path for autostart + ctl
test -x "$root/current/bin/terminator-rust"
test -x "$root/current/bin/terminator-ctl"
echo "current -> $(readlink "$root/current")"
REMOTE
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
Terminal=false
X-GNOME-Autostart-enabled=true
DESK
chmod 0644 "$d/terminator-rust.desktop"
grep -Fx "Exec=$root/current/bin/terminator-rust" "$d/terminator-rust.desktop"
stat -c '%a %U %n' "$d/terminator-rust.desktop"
REMOTE
}

# --- stage 5: restart the app ------------------------------------------------
stage_restart() {
    step "5/6 restart app as $DEPLOY_USER on $HOST"
    # pgrep -f alone would also match the ssh/zsh wrapper carrying this
    # script text; keep only pids whose /proc/<pid>/exe is the installed
    # binary, then kill by pid (never pkill -f).
    run_user "$OPT_ROOT" "$RUNTIME_DIR" <<'REMOTE' || fail "stopping the old instance failed"
root=$1; rt=$2
pids_of() {
    for p in $(pgrep -f "$root/.*/bin/terminator-rust" || true); do
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
        case "$exe" in "$root"/*) echo "$p" ;; esac
    done
}
old=$(pids_of)
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
    run_user "$OPT_ROOT" "$DESKTOP_DISPLAY" "$RUNTIME_DIR" "$LOG_FILE" <<'REMOTE' \
        || fail "app launch failed"
root=$1; disp=$2; rt=$3; log=$4
DISPLAY="$disp" XDG_RUNTIME_DIR="$rt" \
    setsid nohup "$root/current/bin/terminator-rust" \
    >> "$log" 2>&1 </dev/null &
echo "launched on DISPLAY=$disp, log $log"
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
    list_out=$(run_user "$OPT_ROOT" "$RUNTIME_DIR" <<'REMOTE'
root=$1; rt=$2
export XDG_RUNTIME_DIR="$rt"
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
    pane_id=$(run_user "$OPT_ROOT" "$RUNTIME_DIR" <<'REMOTE'
root=$1; rt=$2
export XDG_RUNTIME_DIR="$rt"
"$root/current/bin/terminator-ctl" list --json \
    | grep -o '"id": *[0-9]\+' | head -n1 | grep -o '[0-9]\+'
REMOTE
    ) || fail "could not extract a pane id from 'list --json'"
    [ -n "$pane_id" ] || fail "empty pane id from 'list --json'"
    echo "first pane id: $pane_id"
    cap=$(run_user "$OPT_ROOT" "$RUNTIME_DIR" "$pane_id" <<'REMOTE'
root=$1; rt=$2; pane=$3
export XDG_RUNTIME_DIR="$rt"
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
    echo "PASS: $HOST live via $OPT_ROOT/current ($DIRNAME), autostart installed, list+capture OK"
}

# --- main --------------------------------------------------------------------
step "deploy terminator-rust $VERSION -> $DEPLOY_USER@$HOST ($OPT_ROOT)"
run_root </dev/null || fail "cannot ssh $ROOT_USER@$HOST (key auth?)"
run_user </dev/null || fail "cannot ssh $DEPLOY_USER@$HOST (key auth?)"
stage_build
stage_verify
stage_upload
stage_autostart
stage_restart
stage_smoke
step "done"
