#!/usr/bin/env python3
"""Generate the terminator-rust logo assets (deterministic, Pillow only).

Design intent - the "split T" (concept a1 of the approved prototype): a
dracula-gradient rounded tile carries a glyph of thick round-capped strokes
that reads as the app itself:

    top horizontal bar  = the tab strip                (purple #bd93f9)
    vertical stem       = down from the bar's center   (purple #bd93f9)
    foot crossbar       = the split                    (purple #bd93f9)
    left leg            = pane A                       (pink   #ff79c6)
    right leg           = pane B                       (cyan   #8be9fd)

Palette source: the dracula preset in crates/theme/src/builtin.rs.

Geometry lives on a 1024x1024 design grid (constants below). Rasters render
on a 4x-supersampled master (4096px) and downscale with LANCZOS; sizes <=32px
re-render the glyph with a bolder 112u stroke so it stays legible. Output is
byte-deterministic (no timestamps/randomness): re-runs reproduce the bytes.

Regenerate / verify:
    python3 scripts/bin/gen-logo.py            # write assets/logo/*
    python3 scripts/bin/gen-logo.py --check    # CI gate: fail on byte drift
    python3 scripts/bin/gen-logo.py --preview  # classified ASCII previews
    python3 scripts/bin/gen-logo.py --out DIR  # custom output dir

Writes (default assets/logo/): terminator-rust.svg (vector master),
icon-{16,24,32,48,64,128,256,512,1024}.png (RGBA, FULL-BLEED Linux/Windows
art), icon-256-mac.png (Apple icon-template margin - the raster the app
embeds on macOS), terminator-rust.ico (multi-size 16..256 via PIL),
terminator-rust.icns (hand-built Apple ICNS; every entry carries the same
icon-template margin).
"""

import argparse
import hashlib
import io
import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

# --- design space (1024u grid) -----------------------------------------------
S = 1024
SS = 4                          # supersampling factor of the raster master
W = S * SS                      # master edge (px)
RADIUS = 230                    # tile corner radius (0.225 * S)
STROKE = 96                     # glyph stroke width, design units
STROKE_SMALL = 112              # bolder stroke for icons <= SMALL_MAX px
SMALL_MAX = 32
HAIR_INSET, HAIR_WIDTH, HAIR_ALPHA = 3, 2, 70   # inner hairline
GLOW_ALPHA = 44                 # soft purple glow behind the glyph
GLOW_BOX = ((180, 150), (844, 800))
GLOW_BLUR = 0.10                # gaussian sigma as a fraction of the edge

BG_TOP = (0x34, 0x37, 0x46)     # dracula current-line: gradient top
BG_BOT = (0x21, 0x22, 0x2C)     # dracula background: gradient bottom
PURPLE = (0xBD, 0x93, 0xF9)
PINK = (0xFF, 0x79, 0xC6)
CYAN = (0x8B, 0xE9, 0xFD)
COMMENT = (0x62, 0x72, 0xA4)    # hairline tint
PANE = (0x44, 0x47, 0x5A)       # preview classification only

GLYPH = (                                    # (design (a, b), color)
    (((272, 262), (752, 262)), PURPLE),      # tab-strip bar
    (((512, 262), (512, 512)), PURPLE),      # stem
    (((332, 512), (692, 512)), PURPLE),      # split crossbar
    (((332, 512), (332, 762)), PINK),        # pane A leg
    (((692, 512), (692, 762)), CYAN),        # pane B leg
)

PNG_SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
ICNS_TYPES = (("ic11", 32), ("ic12", 64), ("ic07", 128),
              ("ic08", 256), ("ic09", 512), ("ic10", 1024))

# Apple icon template: macOS scales the WHOLE raster into the icon slot
# (Finder, Dock, and eframe's setApplicationIconImage all publish the
# artwork verbatim), so a full-bleed tile renders ~24% larger than every
# other app's icon. The template body is 824/1024 of the canvas (100u
# margin per side); RADIUS/S == 0.225 already matches Apple's body corner
# ratio, so only the margin is added. Linux/Windows art stays full-bleed.
MAC_BODY = 824
MAC_SIZES = tuple(sorted({s for _, s in ICNS_TYPES}))

# --- raster rendering ---------------------------------------------------------


def sc(x, y):
    """Design units -> master pixels."""
    return (x * SS, y * SS)


def lerp3(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def tile():
    """Master tile: vertical gradient bg + purple glow + inner hairline."""
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    grad = Image.new("RGBA", (W, W))
    gd = ImageDraw.Draw(grad)
    for y in range(W):
        gd.line([(0, y), (W, y)], fill=lerp3(BG_TOP, BG_BOT, y / (W - 1)) + (255,))
    mask = Image.new("L", (W, W), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, W - 1, W - 1], radius=RADIUS * SS, fill=255)
    img.paste(grad, (0, 0), mask)
    glow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse(
        [sc(*GLOW_BOX[0]), sc(*GLOW_BOX[1])], fill=PURPLE + (GLOW_ALPHA,))
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(GLOW_BLUR * W)))
    ins = HAIR_INSET * SS
    ImageDraw.Draw(img).rounded_rectangle(
        [ins, ins, W - 1 - ins, W - 1 - ins],
        radius=(RADIUS - HAIR_INSET) * SS,
        outline=COMMENT + (HAIR_ALPHA,), width=HAIR_WIDTH * SS)
    return img


def stroke(d, a, b, w_u, color):
    """One round-capped thick segment (design coords, width w_u)."""
    (x0, y0), (x1, y1) = sc(*a), sc(*b)
    d.line([x0, y0, x1, y1], fill=color + (255,), width=int(w_u * SS), joint="curve")
    r = w_u * SS / 2
    for x, y in ((x0, y0), (x1, y1)):
        d.ellipse([x - r, y - r, x + r, y + r], fill=color + (255,))


def render(stroke_u):
    """Full master: tile + glyph drawn at the given stroke width."""
    img = tile()
    d = ImageDraw.Draw(img)
    for (a, b), color in GLYPH:
        stroke(d, a, b, stroke_u, color)
    return img


def stroke_for(size):
    return STROKE_SMALL if size <= SMALL_MAX else STROKE


def raster(master_img, size):
    return master_img.resize((size, size), Image.LANCZOS)


def mac_margin(size):
    """Transparent margin (px) of a macOS-template raster of `size`."""
    return size * (S - MAC_BODY) // (2 * S)


def mac_raster(master_img, size):
    """Apple icon-template raster: the master scaled into the body
    (size - 2*margin) and centered on a transparent canvas."""
    m = mac_margin(size)
    body = size - 2 * m
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.alpha_composite(master_img.resize((body, body), Image.LANCZOS), (m, m))
    return out


# --- vector master ------------------------------------------------------------


def hx(c):
    return "#%02x%02x%02x" % c


def svg_text():
    (x0, y0), (x1, y1) = GLOW_BOX
    cx, cy = (x0 + x1) // 2, (y0 + y1) // 2
    rx, ry = (x1 - x0) // 2, (y1 - y0) // 2
    hair = HAIR_INSET + HAIR_WIDTH // 2       # svg strokes straddle the path
    hs = S - 2 * hair
    paths = "\n".join(
        '    <path d="M%d %d L%d %d" stroke="%s"/>'
        % (a[0], a[1], b[0], b[1], hx(c)) for (a, b), c in GLYPH)
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" width="{S}" height="{S}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="{hx(BG_TOP)}"/>
      <stop offset="1" stop-color="{hx(BG_BOT)}"/>
    </linearGradient>
    <clipPath id="tile">
      <rect width="{S}" height="{S}" rx="{RADIUS}" ry="{RADIUS}"/>
    </clipPath>
    <filter id="glow" x="-60%" y="-60%" width="220%" height="220%">
      <feGaussianBlur stdDeviation="{GLOW_BLUR * S:.1f}"/>
    </filter>
  </defs>
  <g clip-path="url(#tile)">
    <rect width="{S}" height="{S}" fill="url(#bg)"/>
    <ellipse cx="{cx}" cy="{cy}" rx="{rx}" ry="{ry}" fill="{hx(PURPLE)}"
             opacity="{GLOW_ALPHA / 255:.3f}" filter="url(#glow)"/>
    <rect x="{hair}" y="{hair}" width="{hs}" height="{hs}" rx="{RADIUS - hair}"
          fill="none" stroke="{hx(COMMENT)}" stroke-opacity="{HAIR_ALPHA / 255:.3f}"
          stroke-width="{HAIR_WIDTH}"/>
  </g>
  <g fill="none" stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round">
{paths}
  </g>
</svg>
'''


# --- icon containers ----------------------------------------------------------


def png_bytes(img):
    buf = io.BytesIO()
    img.save(buf, "PNG")
    return buf.getvalue()


def ico_bytes(frames):
    """Multi-size ICO via PIL; pre-rasterized frames are used verbatim."""
    sizes = sorted(frames)
    buf = io.BytesIO()
    frames[sizes[-1]].save(
        buf, "ICO", sizes=[(s, s) for s in sizes],
        append_images=[frames[s] for s in sizes[:-1]])
    return buf.getvalue()


def icns_bytes(pngs):
    """Hand-built Apple ICNS: magic + BE total + (OSType, BE len, PNG)..."""
    body = b"".join(
        t.encode("ascii") + struct.pack(">I", len(pngs[size]) + 8) + pngs[size]
        for t, size in ICNS_TYPES)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def parse_icns(data):
    """Parse-back validation; returns [(OSType, payload bytes), ...]."""
    assert data[:4] == b"icns", "bad icns magic"
    total = struct.unpack(">I", data[4:8])[0]
    assert total == len(data), f"icns length {total} != file {len(data)}"
    off, seen = 8, []
    while off < total:
        t = data[off:off + 4].decode("ascii")
        n = struct.unpack(">I", data[off + 4:off + 8])[0]
        assert n >= 8 and off + n <= total, f"icns entry {t} out of bounds"
        seen.append((t, data[off + 8:off + n]))
        off += n
    assert off == total, "icns trailing garbage"
    return seen


def build():
    """Render everything; returns (ordered {name: bytes}, {size: Image})."""
    ms = {STROKE: render(STROKE), STROKE_SMALL: render(STROKE_SMALL)}
    rasters = {s: raster(ms[stroke_for(s)], s) for s in PNG_SIZES}
    pngs = {s: png_bytes(rasters[s]) for s in PNG_SIZES}
    mac = {s: mac_raster(ms[stroke_for(s)], s) for s in MAC_SIZES}
    mac_pngs = {s: png_bytes(mac[s]) for s in MAC_SIZES}
    files = {"terminator-rust.svg": svg_text().encode("utf-8")}
    files.update({f"icon-{s}.png": pngs[s] for s in PNG_SIZES})
    # macOS Dock raster (eframe publishes it via setApplicationIconImage).
    files["icon-256-mac.png"] = mac_pngs[256]
    files["terminator-rust.ico"] = ico_bytes({s: rasters[s] for s in ICO_SIZES})
    files["terminator-rust.icns"] = icns_bytes(
        {s: mac_pngs[s] for _, s in ICNS_TYPES})
    return files, rasters


# --- self-verification --------------------------------------------------------

SAMPLES = (                       # (label, design point, expected hue)
    ("bar", (512, 262), "purple"),
    ("stem", (512, 400), "purple"),
    ("crossbar", (430, 512), "purple"),
    ("left-leg", (332, 700), "pink"),
    ("right-leg", (692, 700), "cyan"),
)
CLASS_OF = {"purple": "p", "pink": "k", "cyan": "c"}
PALETTE = ((BG_BOT, "."), (PURPLE, "p"), (PINK, "k"), (CYAN, "c"), (PANE, "n"))
CLASS_MAX_D2 = 6000


def classify(rgb):
    r, g, b = rgb[0], rgb[1], rgb[2]
    ch, best = ".", CLASS_MAX_D2
    for col, c in PALETTE:
        d2 = (col[0] - r) ** 2 + (col[1] - g) ** 2 + (col[2] - b) ** 2
        if d2 < best:
            ch, best = c, d2
    return ch


def hue(rgb):
    """Hue class robust to stroke/bg mixing at tiny sizes (channel order)."""
    r, g, b = rgb[0], rgb[1], rgb[2]
    if b > r > g:
        return "purple"
    if r > b > g:
        return "pink"
    if b > g > r:
        return "cyan"
    return "?"


def px_at(design, size):
    return max(0, min(size - 1, int(round(design * size / S))))


def verify_mac(img, size, label):
    """Assert an Apple icon-template raster of `size`: a fully transparent
    margin ring, an inked body, and the design samples at their
    margin-shifted places."""
    px = img.load()
    m = mac_margin(size)
    assert m > 0, f"{label}: no margin at size {size}"
    body = size - 2 * m
    for i in range(size):                     # (a) margin ring transparent
        for x, y in ((i, m - 1), (i, size - m), (m - 1, i), (size - m, i)):
            assert px[x, y][3] == 0, (
                f"{label}: margin ({x},{y}) alpha {px[x, y][3]} != 0")
    assert px[m + 2, size // 2][3] > 96, f"{label}: body left edge not inked"
    for name, (dx, dy), want in SAMPLES:      # (c) hues inside the body
        x, y = m + px_at(dx, body), m + px_at(dy, body)
        got = px[x, y]
        assert got[3] >= 96, f"{label}: {name} alpha {got[3]}"
        assert hue(got) == want, (
            f"{label}: {name} hue {hue(got)} rgb {got[:3]} != {want}")


def verify(out):
    """Assert the written files are structurally + visually correct."""
    svg = (out / "terminator-rust.svg").read_text("utf-8")
    assert f'viewBox="0 0 {S} {S}"' in svg, "svg viewBox missing"
    assert svg.count("<path") == len(GLYPH), "svg glyph paths missing"
    for size in PNG_SIZES:
        img = Image.open(out / f"icon-{size}.png")
        img.load()
        assert img.mode == "RGBA", f"icon-{size}: mode {img.mode} != RGBA"
        assert img.size == (size, size), f"icon-{size}: size {img.size}"
        c = size // 2
        assert img.getpixel((c, c))[3] > 0, f"icon-{size}: tile center transparent"
        corner = img.getpixel((px_at(4, size), px_at(4, size)))[3]
        if size >= 48:
            assert corner == 0, f"icon-{size}: corner alpha {corner} != 0"
        else:
            # LANCZOS support reaches the tile arc at tiny sizes: near-zero
            assert corner < 16, f"icon-{size}: corner alpha {corner} >= 16"
        for label, (dx, dy), want in SAMPLES:
            got = img.getpixel((px_at(dx, size), px_at(dy, size)))
            assert got[3] >= 96, f"icon-{size}: {label} alpha {got[3]}"
            assert hue(got) == want, (
                f"icon-{size}: {label} hue {hue(got)} rgb {got[:3]} != {want}")
    big = Image.open(out / f"icon-{S}.png")
    big.load()
    for label, (dx, dy), want in SAMPLES:      # exact palette match at 1024
        assert classify(big.getpixel((dx, dy))) == CLASS_OF[want], (
            f"icon-1024: {label} class != {want}")
    ico = Image.open(out / "terminator-rust.ico")
    assert set(ico.info.get("sizes", ())) == {(s, s) for s in ICO_SIZES}, (
        f"ico sizes {sorted(ico.info.get('sizes', ()))}")
    # Linux/Windows art must stay FULL-BLEED (only macOS gets the margin):
    # the tile still reaches the canvas edge at the middle of each side.
    # (LANCZOS kernel clipping at the border costs a little alpha - the
    # measured values are 239/195/254 at 48/256/1024, a mac template is 0.)
    for size in (48, 256, S):
        edge = Image.open(out / f"icon-{size}.png")
        edge.load()
        ep = edge.load()
        for x, y in ((0, size // 2), (size // 2, 0)):
            assert ep[x, y][3] > 128, (
                f"icon-{size}: full-bleed edge ({x},{y}) alpha "
                f"{ep[x, y][3]} <= 128")
    mac = Image.open(out / "icon-256-mac.png")
    mac.load()
    assert mac.mode == "RGBA", f"icon-256-mac: mode {mac.mode} != RGBA"
    assert mac.size == (256, 256), f"icon-256-mac: size {mac.size}"
    verify_mac(mac, 256, "icon-256-mac")
    entries = parse_icns((out / "terminator-rust.icns").read_bytes())
    assert [t for t, _ in entries] == [t for t, _ in ICNS_TYPES], (
        "icns entry order")
    for (ostype, blob), (_, size) in zip(entries, ICNS_TYPES):
        img = Image.open(io.BytesIO(blob))
        img.load()
        assert img.size == (size, size), f"icns {ostype}: size {img.size}"
        assert img.mode == "RGBA", f"icns {ostype}: mode {img.mode} != RGBA"
        verify_mac(img, size, f"icns {ostype}")
    print("verify: OK (svg, 9 full-bleed pngs, mac-template 256 + icns "
          "entries, ico sizes, glyph hues)")


def report(files):
    print(f"{'file':<28}{'bytes':>10}  sha256")
    for name, data in files.items():
        sha = hashlib.sha256(data).hexdigest()[:12]
        print(f"{name:<28}{len(data):>10}  {sha}")


# --- classified ASCII preview -------------------------------------------------


def preview_grid(img, cols):
    """Classify an NxN downscale of img into chars (space = transparent)."""
    small = img.convert("RGBA").resize((cols, cols), Image.LANCZOS)
    px = small.load()
    return ["".join(" " if px[x, y][3] < 40 else classify(px[x, y])
                    for x in range(cols)) for y in range(cols)]


def thin(rows, cap=64):
    stride = max(1, len(rows) // cap)
    return [r[::stride] for r in rows[::stride]]


def preview_text(rasters):
    out = ["classified preview: . bg  p purple(tab bar/stem/split)  "
           "k pink(pane A)  c cyan(pane B)  n pane-gray"]
    for size in (16, 32, 64, 256):
        out.append(f"===== icon-{size}.png "
                   f"({size}x{size}, stroke {stroke_for(size)}u) =====")
        out.extend(thin(preview_grid(rasters[size], size)))
    return "\n".join(out)


# --- CLI ----------------------------------------------------------------------


def default_out():
    return Path(__file__).resolve().parents[2] / "assets" / "logo"


def check_dir(out, files):
    bad = []
    for name, data in files.items():
        p = out / name
        if not p.exists():
            bad.append(f"{name}: MISSING")
        elif p.read_bytes() != data:
            bad.append(f"{name}: bytes differ")
    if out.is_dir():
        extra = sorted(q.name for q in out.iterdir()
                       if q.is_file() and q.name not in files)
        for name in extra:
            print(f"warning: unexpected file {name}", file=sys.stderr)
    return bad


def main(argv=None):
    ap = argparse.ArgumentParser(description="Generate the logo assets.")
    ap.add_argument("--out", default=str(default_out()), help="output directory")
    ap.add_argument("--check", action="store_true",
                    help="fail if the out dir differs from a fresh build")
    ap.add_argument("--preview", action="store_true",
                    help="print classified ASCII previews and exit")
    args = ap.parse_args(argv)
    out = Path(args.out)
    files, rasters = build()
    if args.preview:
        print(preview_text(rasters))
        return 0
    if args.check:
        bad = check_dir(out, files)
        for line in bad:
            print(f"FAIL {line}", file=sys.stderr)
        if bad:
            return 1
        print(f"OK: {len(files)} files in {out} are byte-identical "
              "to a fresh build")
        return 0
    out.mkdir(parents=True, exist_ok=True)
    for name, data in files.items():
        (out / name).write_bytes(data)
    verify(out)
    report(files)
    return 0


if __name__ == "__main__":
    sys.exit(main())
