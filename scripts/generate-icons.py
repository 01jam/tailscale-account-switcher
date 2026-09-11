#!/usr/bin/env python3
"""Render every icon the app ships, with no image library involved.

The same marks as Claude Account Switcher, drawn by the same geometry — this
script is that one with the colours changed — so the two apps sit side by side
in a panel or a dock at exactly the same size:

* **Tray** (`tray.png`, `tray-off.png`) — Tabler's `arrows-exchange`, drawn
  from its two SVG paths (`M7 10h14l-4 -4`, `M17 14h-14l4 4`) with stroke-width
  2 and round caps, scaled to the stroked bounding box rather than the 24x24
  viewBox. White with a thin dark outline: the sibling's terracotta reads on any
  panel, and a plain black or white glyph would vanish on whichever panel, light
  or dark, it was not drawn for. Connected is white; anything else grey.

* **Tray, macOS** (`tray-template.png`, `tray-off-template.png`) — the bare
  glyph as a *template*: the menu bar only looks at alpha and tints it itself.

* **App** (`32x32`, `128x128`, `128x128@2x`, `icon`, and `icon.icns` for the
  macOS bundle) — a black squircle with the arrows in white, tilted 15°, on the
  macOS icon grid: the sibling's icon with black where it has terracotta.
  Nothing is borrowed from Tailscale's own logo, so these regenerate on any
  machine and carry no redistribution caveat.

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
OFF = (156, 154, 152)  # the tray glyph while not connected

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


# --- tray icons -----------------------------------------------------------

TRAY_N = 128
TRAY_MARGIN = 6  # the sibling's: the arrows span the canvas bar this much
# The dark rim around the tray glyph, in viewBox units. It lives inside the
# margin, so the arrows themselves stay exactly the sibling's size; at a 22px
# panel icon it comes out under a pixel — enough to hold white on a light panel.
OUTLINE = 0.8


def render_tray(glyph, outline):
    """The arrows in `glyph` colour, with an `outline` rim or none (templates)."""
    scale = (TRAY_N - 2 * TRAY_MARGIN) / (GX1 - GX0)
    off_x = TRAY_MARGIN
    off_y = (TRAY_N - (GY1 - GY0) * scale) / 2
    segs = [
        (
            ((a[0] - GX0) * scale + off_x, (a[1] - GY0) * scale + off_y),
            ((b[0] - GX0) * scale + off_x, (b[1] - GY0) * scale + off_y),
        )
        for a, b in SEGMENTS
    ]
    radius = STROKE_R * scale
    rim = (STROKE_R + OUTLINE) * scale

    S = 4
    total = S * S
    pixels = []
    for y in range(TRAY_N):
        for x in range(TRAY_N):
            inner = outer = 0
            for sy in range(S):
                for sx in range(S):
                    p = (x + (sx + 0.5) / S, y + (sy + 0.5) / S)
                    d = min(dist_to_segment(p, a, b) for a, b in segs)
                    if d <= radius:
                        inner += 1
                    elif outline is not None and d <= rim:
                        outer += 1
            covered = inner + outer
            if not covered:
                pixels.append((0, 0, 0, 0))
                continue
            if outline is None:
                pixels.append((*glyph[:3], round(255 * glyph[3] * inner / total)))
                continue
            rgb = mix(outline, glyph[:3], inner / covered)
            # The rim is a touch translucent: enough to edge the glyph on a
            # light panel, not enough to thicken it on a dark one.
            alpha = (inner + outer * 0.85) / total
            pixels.append((*rgb, round(255 * alpha)))
    return pixels


# --- app icon -------------------------------------------------------------

GLYPH_ANGLE = math.radians(-15)  # screen coords: negative reads anticlockwise
# Share of the *plate* the tilted glyph spans, measured on its rotated bounding
# box — the sibling's number, so the two icons carry the same weight.
GLYPH_SPAN = 0.62
# macOS draws app icons on a 1024 canvas with the body 824 across: 100px of
# clear space a side, so the app sits at the size of everything around it.
PLATE_MARGIN = 100 / 1024
PLATE_N = 4.5  # superellipse exponent: squircle, not a rounded rectangle


def render_app(size):
    plate_side = size * (1 - 2 * PLATE_MARGIN)
    cos_a, sin_a = math.cos(GLYPH_ANGLE), math.sin(GLYPH_ANGLE)
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
    scale = (plate_side * GLYPH_SPAN) / span
    stroke_r = STROKE_R * scale

    dx = size / 2 - (min(xs) + max(xs)) / 2 * scale
    dy = size / 2 - (min(ys) + max(ys)) / 2 * scale
    segs = [
        ((a[0] * scale + dx, a[1] * scale + dy), (b[0] * scale + dx, b[1] * scale + dy))
        for a, b in unit
    ]

    half = plate_side / 2
    pc = size / 2

    def on_plate(p):
        u, v = abs(p[0] - pc) / half, abs(p[1] - pc) / half
        return u <= 1 and v <= 1 and u**PLATE_N + v**PLATE_N <= 1.0

    S = 3
    total = S * S
    pixels = []
    for y in range(size):
        for x in range(size):
            arrow = plate = 0
            for sy in range(S):
                for sx in range(S):
                    p = (x + (sx + 0.5) / S, y + (sy + 0.5) / S)
                    if not on_plate(p):
                        continue
                    plate += 1
                    if min(dist_to_segment(p, a, b) for a, b in segs) <= stroke_r:
                        arrow += 1

            if not plate:
                pixels.append((0, 0, 0, 0))
                continue

            rgb = mix(INK, PAPER, arrow / total)
            # The plate's own coverage sets alpha, keeping the squircle smooth.
            pixels.append((*rgb, round(255 * plate / total)))
    return pixels


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
    write_png(os.path.join(OUT, "tray.png"), TRAY_N, render_tray((*PAPER, 1.0), INK))
    write_png(os.path.join(OUT, "tray-off.png"), TRAY_N, render_tray((*OFF, 1.0), INK))
    write_png(os.path.join(OUT, "tray-template.png"), TRAY_N, render_tray((*INK, 1.0), None))
    write_png(
        os.path.join(OUT, "tray-off-template.png"), TRAY_N, render_tray((*INK, 0.45), None)
    )

    print("app:")
    for name, size in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ]:
        write_png(os.path.join(OUT, name), size, render_app(size))

    print("macOS bundle:")
    write_icns(os.path.join(OUT, "icon.icns"))


if __name__ == "__main__":
    main()
