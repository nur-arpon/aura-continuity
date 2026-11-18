"""Draw the Continuity icons (navy / blue only) with the standard library: no PIL needed.

    python tools/make_icons.py      (run from pc/tray)

Writes icons/icon.png (256), 128x128.png, 32x32.png, icon.ico and the three tray states.
Colours are the placeholder tokens from ui/tokens.css (navy-900 / blue-500 / blue-300).
"""
import os, struct, zlib, math

NAVY = (11, 27, 58)       # --navy-900 #0B1B3A
BLUE = (47, 111, 235)     # --blue-500 #2F6FEB
LIGHT = (142, 182, 255)   # --blue-300 #8EB6FF
DIM = (74, 96, 140)       # --navy-400 #4A608C

def png(w, h, px):
    raw = b"".join(b"\x00" + bytes(c for p in px[y*w:(y+1)*w] for c in p) for y in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")

def render(size, ring, dot, bg=True):
    ss = 4
    S = size * ss
    acc = [[0, 0, 0, 0] for _ in range(size * size)]
    r_bg = 0.22 * S
    c1 = (0.40 * S, 0.5 * S); c2 = (0.60 * S, 0.5 * S); rr = 0.20 * S; th = 0.065 * S
    for y in range(S):
        for x in range(S):
            col = None
            if bg:
                # rounded square
                dx = max(abs(x + .5 - S/2) - (S/2 - r_bg), 0); dy = max(abs(y + .5 - S/2) - (S/2 - r_bg), 0)
                if dx*dx + dy*dy <= r_bg*r_bg:
                    col = NAVY
            for (cx, cy), c in ((c1, ring), (c2, dot)):
                d = math.hypot(x + .5 - cx, y + .5 - cy)
                if abs(d - rr) <= th:
                    col = c
            if col:
                a = acc[(y // ss) * size + x // ss]
                a[0] += col[0]; a[1] += col[1]; a[2] += col[2]; a[3] += 1
    n = ss * ss
    return [(a[0] // a[3], a[1] // a[3], a[2] // a[3], 255 * a[3] // n) if a[3] else (0, 0, 0, 0) for a in acc]

def ico(pngs):
    head = struct.pack("<HHH", 0, 1, len(pngs))
    off = 6 + 16 * len(pngs); dirs = b""; data = b""
    for s, b in pngs:
        dirs += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(b), off + len(data)); data += b
    return head + dirs + data

os.makedirs("icons", exist_ok=True)
sizes = {}
for s in (16, 32, 48, 128, 256):
    sizes[s] = png(s, s, render(s, LIGHT, BLUE))
open("icons/icon.png", "wb").write(sizes[256])
open("icons/128x128.png", "wb").write(sizes[128])
open("icons/32x32.png", "wb").write(sizes[32])
open("icons/icon.ico", "wb").write(ico([(s, sizes[s]) for s in (16, 32, 48, 256)]))
for name, ring, dot in (("connected", LIGHT, BLUE), ("reconnecting", DIM, LIGHT), ("offline", DIM, DIM)):
    open(f"icons/tray-{name}.png", "wb").write(png(32, 32, render(32, ring, dot)))
print("icons written")
