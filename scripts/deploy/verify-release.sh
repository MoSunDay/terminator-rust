#!/usr/bin/env bash
# Validate the exact archive to be uploaded, using only private test state.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "$#" -eq 1 ] || { echo "usage: verify-release.sh ARCHIVE" >&2; exit 2; }
ARCHIVE=$(realpath "$1")
ARTIFACT=$(sha256sum "$ARCHIVE" | cut -d ' ' -f1)
VERIFY_ROOT=$(mktemp -d /tmp/terminator-release-verify-XXXXXX)
trap 'rm -rf "$VERIFY_ROOT"' EXIT
python3 scripts/deploy/release.py "$VERIFY_ROOT" preflight
cp "$ARCHIVE" "$VERIFY_ROOT/incoming/$ARTIFACT.tar.gz"
python3 scripts/deploy/release.py "$VERIFY_ROOT" install "$ARTIFACT"
scripts/bin/e2e-ime.sh --bundle "$VERIFY_ROOT/current"
[ "$(sha256sum "$ARCHIVE" | cut -d ' ' -f1)" = "$ARTIFACT" ] || {
    echo "archive changed during validation" >&2; exit 1;
}
echo "verified release artifact: $ARTIFACT"
