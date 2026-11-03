#!/usr/bin/env bash
# Validate the exact archive to be uploaded, using only private test state.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "$#" -eq 1 ] || { echo "usage: verify-release.sh ARCHIVE" >&2; exit 2; }
ARCHIVE=$(realpath "$1")
SIDECAR="$ARCHIVE.sha256"
[ -f "$SIDECAR" ] || { echo "verify-release: missing sha256 sidecar: $SIDECAR" >&2; exit 2; }
( cd "$(dirname "$ARCHIVE")" && sha256sum -c --status "$(basename "$SIDECAR")" ) \
    || { echo "verify-release: sidecar does not match $ARCHIVE" >&2; exit 1; }
ARTIFACT=$(sha256sum "$ARCHIVE" | cut -d ' ' -f1)
# The sidecar must bind THIS archive: parse its first line and require the
# recorded filename to be the archive's basename and the hash to be the
# freshly computed one (a sidecar renamed alongside the wrong tarball would
# otherwise only be caught by the -c run above, which trusts the filename
# it is handed). sha256sum marks binary-mode lines with a leading '*'.
SIDECAR_LINE=$(head -n 1 "$SIDECAR")
SIDECAR_HASH=$(printf '%s\n' "$SIDECAR_LINE" | cut -d ' ' -f1)
SIDECAR_NAME=$(printf '%s\n' "$SIDECAR_LINE" | cut -d ' ' -f3)
SIDECAR_NAME=${SIDECAR_NAME#\*}
BASENAME=$(basename "$ARCHIVE")
[ "$SIDECAR_NAME" = "$BASENAME" ] \
    || { echo "verify-release: sidecar names '$SIDECAR_NAME', expected '$BASENAME'" >&2; exit 1; }
[ "$SIDECAR_HASH" = "$ARTIFACT" ] \
    || { echo "verify-release: sidecar hash $SIDECAR_HASH != archive hash $ARTIFACT" >&2; exit 1; }
VERIFY_ROOT=$(mktemp -d /tmp/terminator-release-verify-XXXXXX)
trap 'rm -rf "$VERIFY_ROOT"' EXIT
step() { echo "== $*"; }

# Hermetic gates: caller exports must not weaken the release checks. The
# idle gate deliberately asserts the CPU governor is ENABLED (default
# target, no explicit frame floor); APP_BIN/CTL_BIN are set explicitly
# below so the script never builds from the working tree.
run_hermetic() {
    env -i PATH="$PATH" HOME="$HOME" TMPDIR="${TMPDIR:-/tmp}" \
        LANG="${LANG:-C.UTF-8}" TERM=dumb "$@"
}

step "install $ARTIFACT"
python3 scripts/deploy/release.py "$VERIFY_ROOT" preflight
cp "$ARCHIVE" "$VERIFY_ROOT/incoming/$ARTIFACT.tar.gz"
python3 scripts/deploy/release.py "$VERIFY_ROOT" install "$ARTIFACT"

# Runtime gate 1: a terminal that is idle must not repaint (0 fps) and
# must stay under 1% of one core (reactive render loop contract, S1 in
# agents/perf/index.md), and S14 must run with the CPU governor ENABLED.
# The environment is scrubbed: APP_BIN/CTL_BIN override paths, IDLE_*
# tuning knobs and the TERMINATOR_FRAME_FLOOR_MS/CPU_TARGET_PCT governor
# switches never leak in from the caller.
step "idle CPU + governor (e2e-idle-cpu.sh, hermetic env)"
run_hermetic \
    "APP_BIN=$VERIFY_ROOT/current/bin/terminator-rust" \
    "CTL_BIN=$VERIFY_ROOT/current/bin/terminator-ctl" \
    scripts/bin/e2e-idle-cpu.sh

# Runtime gate 2: the IME/latch behaviour of the installed bundle.
step "IME suite (e2e-ime.sh --bundle, hermetic env)"
run_hermetic scripts/bin/e2e-ime.sh --bundle "$VERIFY_ROOT/current"

[ "$(sha256sum "$ARCHIVE" | cut -d ' ' -f1)" = "$ARTIFACT" ] || {
    echo "archive changed during validation" >&2; exit 1;
}
echo "verified release artifact: $ARTIFACT"
