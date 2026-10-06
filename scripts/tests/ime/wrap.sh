#!/usr/bin/env bash
# The existing raw PTY fixture is still running after native.sh's commit.
step "M3e: exact row ends, wide input and bottom-row preview scroll"
for mode in e i l; do
    "$CTL" send "$PANE" --text "$mode" >/dev/null
    sleep .3
    scrot "$ROOT/edge-$mode-base.png"
    xdotool type --delay 40 ni
    sleep .4
    python3 scripts/tests/ime/assertions.py snapshot "$ROOT/edge-$mode-compose.png"
    xdotool key --clearmodifiers Escape
    sleep .3
    scrot "$ROOT/edge-$mode-cancel.png"
    python3 scripts/tests/ime/assertions.py edge "$ROOT" "$mode"
done
xdotool type --delay 40 ni
xdotool key --clearmodifiers space
sleep .3
cap > "$ROOT/edge-commit.txt"
python3 - "$ROOT/edge-commit.txt" <<'PY'
from pathlib import Path
import sys
lines = Path(sys.argv[1]).read_text().splitlines()
prefix = next(i for i, line in enumerate(lines) if '汉字' in line)
assert lines[prefix + 1].startswith('你'), 'commit did not follow the previewed wrap'
print('M3e: real commit lands after the preserved previous line')
PY
