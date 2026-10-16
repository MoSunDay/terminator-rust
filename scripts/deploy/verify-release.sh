#!/usr/bin/env bash
# Validate the exact archive to be uploaded, using only private test state.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "$#" -eq 1 ] || { echo "usage: verify-release.sh ARCHIVE" >&2; exit 2; }
ARCHIVE=$(realpath "$1")
ARTIFACT=$(sha256sum "$ARCHIVE" | cut -d ' ' -f1)
VERIFY_ROOT=$(mktemp -d /tmp/terminator-release-verify-XXXXXX)
trap 'rm -rf "$VERIFY_ROOT"' EXIT
step() { echo "== $*"; }

step "install $ARTIFACT"
python3 scripts/deploy/release.py "$VERIFY_ROOT" preflight
cp "$ARCHIVE" "$VERIFY_ROOT/incoming/$ARTIFACT.tar.gz"
python3 scripts/deploy/release.py "$VERIFY_ROOT" install "$ARTIFACT"

# Runtime gate 1: a terminal that is idle must not repaint (0 fps) and
# must stay under 1% of one core (reactive render loop contract, S1 in
# agents/perf/index.md). APP_BIN/CTL_BIN point the gate at binaries
# other than the installed bundle (e.g.
# APP_BIN=/usr/local/bin/terminator-rust for a live install); a caller
# that exports them here overrides this run. IDLE_MAX1S_PCT / IDLE_MAX_FPS
# / IDLE_SAMPLE_S pass through to the gate for hosts with a known floor
# (software Vulkan under Xvfb measures a few percent with zero frames).
step "idle CPU (e2e-idle-cpu.sh)"
APP_BIN="${APP_BIN:-$VERIFY_ROOT/current/bin/terminator-rust}" \
CTL_BIN="${CTL_BIN:-$VERIFY_ROOT/current/bin/terminator-ctl}" \
    scripts/bin/e2e-idle-cpu.sh

# Runtime gate 2: the IME/latch behaviour of the installed bundle.
step "IME suite (e2e-ime.sh --bundle)"
scripts/bin/e2e-ime.sh --bundle "$VERIFY_ROOT/current"

[ "$(sha256sum "$ARCHIVE" | cut -d ' ' -f1)" = "$ARTIFACT" ] || {
    echo "archive changed during validation" >&2; exit 1;
}
echo "verified release artifact: $ARTIFACT"
