#!/usr/bin/env python3
"""Render every icon the app ships, with no image library involved.

Three families, all black and white:

* **Tray, Linux** (`tray.png`, `tray-off.png`) — Tabler's `arrows-exchange`.
  Connected is the glyph in white on a black squircle: a single-colour glyph on
  a transparent ground would vanish on whichever panel, light or dark, it was
  not drawn for, and a white glyph on its own plate reads on both. Anything
  else is the plate's outline and the glyph in grey — weaker than connected on
  either panel, which is the whole message.

* **Tray, macOS** (`tray-template.png`, `tray-off-template.png`) — the bare
  glyph as a *template*: the menu bar only looks at alpha and tints it itself,
  so a plate would turn into a solid blob there.

* **App** (`32x32`, `128x128`, `128x128@2x`, `icon`, and `icon.icns` for the
  macOS bundle) — a black squircle with the same glyph in white, tilted 15°,
  on the macOS icon grid. The sibling Claude Account Switcher draws the very
  same mark on terracotta; nothing is borrowed from Tailscale's own logo, so
  these regenerate on any machine and carry no redistribution caveat.

Run: python3 scripts/generate-icons.py
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "src-tauri", "icons")

INK = (31, 30, 30)  # the near-black of the UI's --text
PAPER = (255, 255, 255)
OFF_GREY = (140, 138, 136)  # holds on light and dark panels alike
# A hairline just inside the app plate's edge. Black on a dark dock is no edge
# at all, and the icon would read as loose arrows.
EDGE = (70, 68, 66)

# --- arrows-exchange geometry -------------------------------------------

SEGMENTS = [
    ((7, 10), (21, 10)),
    ((21, 10), (17, 6)),
    ((17, 14), (3, 14)),
    ((3, 14), (7, 18)),
]
STROKE_R = 1.0
GX0, GX1 = 3 - STROKE_R, 21 + STROKE_R
GY0, GY1 = 6 - STROKE_R, 18 + STROKE_R


def dist_to_segment(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    length2 = dx * dx + dy * dy
    if length2 == 0:
        return math.hypot(p[0] - a[0], p[1] - a[1])
    t = max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2))
    return math.hypot(p[0] - (a[0] + t * dx), p[1] - (a[1] + t * dy))


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


# --- PNG ------------------------------------------------------------------


def write_png(path, size, pixels):
    raw = bytearray()
    for y in range(size):
        raw.append(0)
        for x in range(size):
            raw.extend(pixels[y * size + x])

    def chunk(tag, body):
        return (
            struct.pack(">I", len(body))
            + tag
            + body
            + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)
        )

    blob = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as f:
        f.write(blob)
    print(f"  {os.path.basename(path):24} {size:>4}px  {len(blob):>6} bytes")


# --- shared renderer ----------------------------------------------------------

PLATE_N = 4.5  # superellipse exponent: squircle, not a rounded rectangle


def render(
    size, *, plate_margin, glyph_span, angle, plate, glyph_alpha, samples, ring=None, edge=None
):
    """A glyph centred on an optional squircle plate.

    `plate` is the plate colour, or None for a bare glyph (the template icons).
    `ring` draws only the plate's outline, that thick as a share of the plate,
    with plate and glyph both in the `plate` colour. `edge` is a hairline
    colour just inside a filled plate's border. `glyph_span` is the share of the
    plate — or of the canvas, with no plate — that the glyph's rotated bounding
    box spans.
    """
    plate_side = size * (1 - 2 * plate_margin)
    cos_a, sin_a = math.cos(angle), math.sin(angle)
    cx, cy = (GX0 + GX1) / 2, (GY0 + GY1) / 2

    def place(p):
        x, y = p[0] - cx, p[1] - cy
        return (x * cos_a - y * sin_a, x * sin_a + y * cos_a)

    # Sized and centred on the extent the mark has once tilted, not on the
    # upright box: the arrows run corner to corner, so the tilt pulls them in.
    unit = [(place(a), place(b)) for a, b in SEGMENTS]
    xs = [p[0] for seg in unit for p in seg]
    ys = [p[1] for seg in unit for p in seg]
    span = max(max(xs) - min(xs), max(ys) - min(ys)) + 2 * STROKE_R
    scale = (plate_side * glyph_span) / span
    stroke_r = STROKE_R * scale

    dx = size / 2 - (min(xs) + max(xs)) / 2 * scale
    dy = size / 2 - (min(ys) + max(ys)) / 2 * scale
    segs = [
        ((a[0] * scale + dx, a[1] * scale + dy), (b[0] * scale + dx, b[1] * scale + dy))
        for a, b in unit
    ]

    half = plate_side / 2
    pc = size / 2

    def inside(p, h):
        u, v = abs(p[0] - pc) / h, abs(p[1] - pc) / h
        return u <= 1 and v <= 1 and u**PLATE_N + v**PLATE_N <= 1.0

    def on_plate(p):
        return inside(p, half)

    inner = half - (ring or 0) * plate_side
    edge_inner = half - max(1.0, plate_side * 0.012)

    S = samples
    total = S * S
    pixels = []
    for y in range(size):
        for x in range(size):
            arrow = covered = rim = 0
            for sy in range(S):
                for sx in range(S):
                    p = (x + (sx + 0.5) / S, y + (sy + 0.5) / S)
                    hit = min(dist_to_segment(p, a, b) for a, b in segs) <= stroke_r
                    if plate is None:
                        if hit:
                            arrow += 1
                        continue
                    if not on_plate(p):
                        continue
                    if ring is not None:
                        if hit or not inside(p, inner):
                            covered += 1
                        continue
                    covered += 1
                    if hit:
                        arrow += 1
                    elif edge is not None and not inside(p, edge_inner):
                        rim += 1

            if plate is None:
                # Template: only alpha matters to the menu bar.
                pixels.append((*INK, round(255 * glyph_alpha * arrow / total)))
                continue
            if not covered:
                pixels.append((0, 0, 0, 0))
                continue
            if ring is not None:
                pixels.append((*plate, round(255 * covered / total)))
                continue
            rgb = mix(plate, PAPER, glyph_alpha * arrow / covered)
            if rim:
                rgb = mix(rgb, edge, rim / covered)
            pixels.append((*rgb, round(255 * covered / total)))
    return pixels


# --- tray icons -----------------------------------------------------------

TRAY_N = 128


def tray(plate, ring=None):
    # Nearly edge to edge: a panel icon is 16-24px, and every pixel of margin
    # at that size is a pixel of glyph lost. Upright, unlike the app icon —
    # tilted strokes that thin go soft.
    return render(
        TRAY_N,
        plate_margin=4 / TRAY_N,
        glyph_span=0.72,
        angle=0.0,
        plate=plate,
        glyph_alpha=1.0,
        samples=4,
        ring=ring,
    )


def tray_template(glyph_alpha):
    return render(
        TRAY_N,
        plate_margin=0.0,
        glyph_span=0.94,
        angle=0.0,
        plate=None,
        glyph_alpha=glyph_alpha,
        samples=4,
    )


# --- app icon -------------------------------------------------------------

# macOS draws app icons on a 1024 canvas with the body 824 across: 100px of
# clear space a side, so the app sits at the size of everything around it.
APP_MARGIN = 100 / 1024
APP_GLYPH_SPAN = 0.62
APP_ANGLE = math.radians(-15)  # screen coords: negative reads anticlockwise


def app(size):
    return render(
        size,
        plate_margin=APP_MARGIN,
        glyph_span=APP_GLYPH_SPAN,
        angle=APP_ANGLE,
        plate=INK,
        glyph_alpha=1.0,
        samples=3,
        edge=EDGE,
    )


# --- macOS bundle icon ---------------------------------------------------

# An .icns is a container: a header, then one record per size holding the PNG
# verbatim. `ic09`/`ic14` and `ic08`/`ic13` carry the same pixels at a nominal
# size and its retina equivalent, which is how macOS wants them.
ICNS_ENTRIES = [
    (b"icp5", "32x32.png"),
    (b"ic07", "128x128.png"),
    (b"ic08", "128x128@2x.png"),
    (b"ic13", "128x128@2x.png"),
    (b"ic09", "icon.png"),
    (b"ic14", "icon.png"),
]


def write_icns(path):
    body = b""
    for kind, name in ICNS_ENTRIES:
        src = os.path.join(OUT, name)
        if not os.path.exists(src):
            continue
        with open(src, "rb") as f:
            data = f.read()
        body += kind + struct.pack(">I", len(data) + 8) + data

    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(body) + 8) + body)
    print(f"  {os.path.basename(path)}")


def main():
    os.makedirs(OUT, exist_ok=True)

    print("tray:")
    write_png(os.path.join(OUT, "tray.png"), TRAY_N, tray(INK))
    write_png(os.path.join(OUT, "tray-off.png"), TRAY_N, tray(OFF_GREY, ring=0.075))
    write_png(os.path.join(OUT, "tray-template.png"), TRAY_N, tray_template(1.0))
    write_png(os.path.join(OUT, "tray-off-template.png"), TRAY_N, tray_template(0.45))

    print("app:")
    for name, size in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ]:
        write_png(os.path.join(OUT, name), size, app(size))

    print("macOS bundle:")
    write_icns(os.path.join(OUT, "icon.icns"))


if __name__ == "__main__":
    main()
