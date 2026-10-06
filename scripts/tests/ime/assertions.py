#!/usr/bin/env python3
"""Measure the real IBus window and app preedit, excluding panel highlight ink."""
import json
from pathlib import Path
import subprocess
import sys

from PIL import Image


def geometry(window):
    output = subprocess.check_output(['xdotool', 'getwindowgeometry', '--shell', window], text=True)
    return {key: int(value) for key, value in (line.split('=') for line in output.splitlines())}


def panel():
    output = subprocess.run(['xdotool', 'search', '--onlyvisible', '--class', 'ibus-ui-gtk3'],
                            capture_output=True, text=True)
    boxes = [geometry(window) for window in output.stdout.split()]
    boxes = [box for box in boxes if box['WIDTH'] > 40 and box['HEIGHT'] > 20]
    assert len(boxes) == 1, 'expected one visible native candidate window: ' + repr(boxes)
    return boxes[0]


def contains(box, x, y):
    return box['X'] <= x < box['X'] + box['WIDTH'] and box['Y'] <= y < box['Y'] + box['HEIGHT']


def blue(rgb):
    r, g, b = rgb
    return b - g >= 18 and r - g <= 5 and b >= 110


def red(rgb):
    r, g, b = rgb
    return r > g + 40 and r > b + 20


def snapshot(path):
    box = panel()
    subprocess.run(['scrot', str(path)], check=True)
    image = Image.open(path).convert('RGB')
    points = [(x, y) for y in range(65, image.height) for x in range(image.width)
              if not contains(box, x, y) and blue(image.getpixel((x, y)))]
    assert len(points) >= 4, 'app preedit underline is missing'
    bounds = [min(x for x, y in points), min(y for x, y in points),
              max(x for x, y in points), max(y for x, y in points)]
    result = {'panel': box, 'underline': bounds}
    Path(str(path) + '.json').write_text(json.dumps(result))
    print(path.name, result, flush=True)
    # The default fixture has 20px cells. The underline's last pixel is the
    # baseline; its first pixel can be one pixel above it due to antialiasing.
    # Comparing against that first pixel incorrectly rejects an adjacent panel.
    row_top = bounds[3] - 20
    assert box['Y'] > bounds[3] or box['Y'] + box['HEIGHT'] <= row_top, \
        'native candidate window overlaps the composing row'
    distance = min(abs(box['Y'] - bounds[3] - 1),
                   abs(box['Y'] + box['HEIGHT'] - row_top))
    assert distance <= 24, 'native candidate window did not follow the insertion row'
    return result


def delayed(compose, cancel):
    comp = Image.open(compose).convert('RGB')
    base = Image.open(cancel).convert('RGB')
    box = json.loads(Path(str(compose) + '.json').read_text())
    underline = box['underline']
    points = [(x, y) for y in range(max(65, underline[1] - 24), underline[1])
              for x in range(comp.width) if red(base.getpixel((x, y)))]
    assert len(points) >= 30, 'committed prefix is missing'
    assert all(red(comp.getpixel(p)) for p in points), 'next preedit covers committed glyphs'
    assert underline[0] > max(x for x, y in points), 'preedit must follow the committed prefix'
    print('Delayed echo:', len(points), 'committed glyph pixels stay visible')


def edge(root, mode):
    base, comp, cancel = [Image.open(root / f'edge-{mode}-{phase}.png').convert('RGB')
                          for phase in ('base', 'compose', 'cancel')]
    points = [(x, y) for y in range(65, base.height) for x in range(base.width)
              if red(base.getpixel((x, y)))]
    assert len(points) > 30, 'edge fixture is missing'
    offset = -20 if mode == 'l' else 0
    # Moving a glyph by one row can change antialiasing by one RGB level.
    # Compare its color to the original instead of dropping threshold-edge pixels.
    assert all(max(abs(a - b) for a, b in zip(base.getpixel((x, y)),
                                             comp.getpixel((x, y + offset)))) <= 2
               for x, y in points), \
        'preedit or candidate panel covers the committed row-end text'
    assert all(red(cancel.getpixel(p)) for p in points), 'cancel changed terminal contents'
    bounds = json.loads(Path(str(root / f'edge-{mode}-compose.png') + '.json').read_text())['underline']
    assert bounds[0] < 20, 'next preedit must start at the first column'
    assert bounds[3] > max(y + offset for x, y in points), 'preedit must follow the prefix row'
    print(f'Row-end {mode}: all {len(points)} committed glyph pixels remain visible; cancel restores them')


if __name__ == '__main__':
    if sys.argv[1] == 'snapshot':
        snapshot(Path(sys.argv[2]))
    elif sys.argv[1] == 'delayed':
        delayed(Path(sys.argv[2]), Path(sys.argv[3]))
    elif sys.argv[1] == 'edge':
        edge(Path(sys.argv[2]), sys.argv[3])
    else:
        raise SystemExit('unknown assertion')
