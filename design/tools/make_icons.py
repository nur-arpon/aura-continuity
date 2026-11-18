"""Continuity icon builder: writes the SVG sources and renders every PNG/ICO.

    python design\\tools\\make_icons.py

Sources land in design\\icons\\*.svg (edit the geometry HERE, not the SVGs:
they are generated so the family stays consistent). Rendering uses Playwright's
bundled Chromium (already on this machine), so the SVGs are rasterised by a
real browser engine with transparent backgrounds.

Pixel-grid rule: the 16 px and 24 px app icons and the tray icons are drawn on
their own grids (whole / half pixels), not scaled down from 256, so they stay
crisp. Tray PNGs are 48 px drawn on a 24 grid: exact 2:1 at 150 % scaling
(this laptop's 24 px tray slot), 1.5:1 at 200 %, 3:1 at 100 %.
"""
import io, math, os, sys
from pathlib import Path
from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]          # D:\Claude-Projects\Continuity
SRC = ROOT / "design" / "icons"
APPS = {"autolock": ROOT / "autolock" / "src-tauri" / "icons", "handoff": ROOT / "handoff" / "src-tauri" / "icons"}

# ---- palette (matches design\tokens.css) ----
NAVY_TOP, NAVY_BOT = "#1F4C8F", "#0B1B38"     # app tile
BLUE_TOP, BLUE_BOT = "#4A98FF", "#1D5FD0"     # "active" tile (live / sending)
SLATE_TOP, SLATE_BOT = "#33415C", "#18223A"   # paused tile
GLYPH = "#EEF4FF"
GLYPH_DIM = "#B4C3DB"
ARC_A, ARC_B = "#9AD0FF", "#3D8BFF"
INK = "#0F2448"                                # keyhole / speaker cut-outs
AMBER = "#F3B95F"


def svg(view, body, defs=""):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view} {view}" width="{view}" height="{view}">'
            f"<defs>{defs}</defs>{body}</svg>")


def grad(id_, a, b, x2="0", y2="1"):
    return (f'<linearGradient id="{id_}" x1="0" y1="0" x2="{x2}" y2="{y2}">'
            f'<stop offset="0" stop-color="{a}"/><stop offset="1" stop-color="{b}"/></linearGradient>')


def tile(view, inset, radius, top, bot, edge=True):
    s = view - 2 * inset
    out = f'<rect x="{inset}" y="{inset}" width="{s}" height="{s}" rx="{radius}" fill="url(#tile)"/>'
    if edge:  # 1-unit inner highlight, gives the tile a lit top edge
        out += (f'<rect x="{inset + .5 * view / 256 * 2}" y="{inset + .5 * view / 256 * 2}" width="{s - view / 256 * 2}" '
                f'height="{s - view / 256 * 2}" rx="{radius - view / 256}" fill="none" stroke="#fff" '
                f'stroke-opacity=".12" stroke-width="{view / 256 * 2}"/>')
    return grad("tile", top, bot) + "|" + out


def build(view, inset, radius, top, bot, glyph, edge=True, extra_defs=""):
    d, t = tile(view, inset, radius, top, bot, edge).split("|")
    return svg(view, t + glyph, d + extra_defs)


def arc_path(cx, cy, r, a0, a1):
    """Clockwise arc from angle a0 to a1 (degrees, 0 = up)."""
    p = lambda a: (cx + r * math.sin(math.radians(a)), cy - r * math.cos(math.radians(a)))
    (x0, y0), (x1, y1) = p(a0), p(a1)
    large = 1 if (a1 - a0) % 360 > 180 else 0
    return f"M{x0:.2f} {y0:.2f}A{r} {r} 0 {large} 1 {x1:.2f} {y1:.2f}", p


# ------------------------------------------------------------------ glyphs
def lock_256(cx=128, cy=132, fill=GLYPH, ink=INK):
    return (f'<path d="M{cx-28} {cy-8}V{cy-28}a28 28 0 0 1 56 0V{cy-8}" fill="none" stroke="{fill}" stroke-width="15"/>'
            f'<rect x="{cx-44}" y="{cy-8}" width="88" height="68" rx="16" fill="{fill}"/>'
            f'<circle cx="{cx}" cy="{cy+20}" r="9" fill="{ink}"/><rect x="{cx-4.5}" y="{cy+22}" width="9" height="18" rx="4.5" fill="{ink}"/>')


def phone_256(cx=128, cy=128, fill=GLYPH, ink=INK):
    return (f'<rect x="{cx-34}" y="{cy-58}" width="68" height="116" rx="16" fill="{fill}"/>'
            f'<rect x="{cx-24}" y="{cy-46}" width="48" height="80" rx="6" fill="{ink}"/>'
            f'<rect x="{cx-10}" y="{cy+42}" width="20" height="6" rx="3" fill="{ink}"/>'
            # something arriving on the screen
            f'<path d="M{cx-14} {cy-6}h22M{cx-2} {cy-18}l12 12-12 12" fill="none" stroke="{ARC_A}" '
            f'stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/>')


def ring_256(kind):
    """The family signature: a bright-blue orbit around the glyph.
    AutoLock ends it with a dot (your device, nearby); Handoff with an arrowhead (on its way)."""
    # Same orbit + dot on every Continuity app: the dot is "your other device".
    # The glyph in the middle says which app it is.
    r, w = 94, 11
    d, p = arc_path(128, 128, r, 60, 380 - 20)   # gap at the upper right for the device dot
    x, y = p(40)
    return (f'<path d="{d}" fill="none" stroke="url(#arc)" stroke-width="{w}" stroke-linecap="round"/>'
            f'<circle cx="{x:.2f}" cy="{y:.2f}" r="13" fill="{ARC_A}"/>')


ARC_DEFS = grad("arc", ARC_A, ARC_B, "1", "1")


def app_master(kind):
    glyph = lock_256() if kind == "autolock" else phone_256()
    return build(256, 12, 56, NAVY_TOP, NAVY_BOT, ring_256(kind) + glyph, extra_defs=ARC_DEFS)


# 24 px app icon: ring at 2 px, glyph snapped to the half-pixel grid.
def app_24(kind):
    if kind == "autolock":
        g = ('<path d="M10.5 3.6a8.75 8.75 0 1 0 7.9 3.4" fill="none" stroke="url(#arc)" stroke-width="1.6" stroke-linecap="round"/>'
             f'<circle cx="16.6" cy="4.9" r="1.5" fill="{ARC_A}"/>'
             f'<path d="M10 11V9.5a2 2 0 0 1 4 0V11" fill="none" stroke="{GLYPH}" stroke-width="1.5"/>'
             f'<rect x="8" y="11" width="8" height="6" rx="1.5" fill="{GLYPH}"/>'
             f'<rect x="11.25" y="13" width="1.5" height="2.25" rx=".75" fill="{INK}"/>')
    else:
        g = ('<path d="M10.5 3.6a8.75 8.75 0 1 0 7.9 3.4" fill="none" stroke="url(#arc)" stroke-width="1.6" stroke-linecap="round"/>'
             f'<circle cx="16.6" cy="4.9" r="1.5" fill="{ARC_A}"/>'
             f'<rect x="9" y="6.5" width="6" height="11" rx="1.5" fill="{GLYPH}"/>'
             f'<rect x="10" y="8" width="4" height="7" rx=".5" fill="{INK}"/>')
    return build(24, 1, 6, NAVY_TOP, NAVY_BOT, g, edge=False, extra_defs=ARC_DEFS)


# 16 px app icon: no ring (it would be a 1 px smear); the bright dot / arrow keeps the family cue.
def app_16(kind):
    if kind == "autolock":
        g = (f'<path d="M6.5 7.5V6a1.5 1.5 0 0 1 3 0v1.5" fill="none" stroke="{GLYPH}" stroke-width="1.25"/>'
             f'<rect x="4.5" y="7.5" width="7" height="5" rx="1" fill="{GLYPH}"/>'
             f'<rect x="7.5" y="9" width="1" height="2" fill="{INK}"/>'
             f'<circle cx="12.5" cy="3.5" r="1.5" fill="{ARC_A}"/>')
    else:
        g = (f'<rect x="5" y="3" width="6" height="10" rx="1" fill="{GLYPH}"/>'
             f'<rect x="6" y="4" width="4" height="7" fill="{INK}"/>'
             f'<circle cx="12.5" cy="3.5" r="1.5" fill="{ARC_A}"/>')
    return build(16, 0, 4, NAVY_TOP, NAVY_BOT, g, edge=False, extra_defs=ARC_DEFS)


# ------------------------------------------------------------------ tray (24 grid)
def tray(kind, state):
    top, bot = NAVY_TOP, NAVY_BOT
    if (kind, state) in (("autolock", "live"), ("handoff", "sending")):
        top, bot = BLUE_TOP, BLUE_BOT
    if state == "paused":
        top, bot = SLATE_TOP, SLATE_BOT
    if kind == "autolock":
        if state == "dry":      # hollow lock: watching, but it will not really lock
            g = (f'<path d="M9.5 10.25V8.5a2.5 2.5 0 0 1 5 0v1.75" fill="none" stroke="{GLYPH}" stroke-width="2"/>'
                 f'<rect x="7" y="10.75" width="10" height="7.5" rx="1.75" fill="none" stroke="{GLYPH}" stroke-width="1.5"/>'
                 f'<circle cx="12" cy="14.5" r="1.25" fill="{GLYPH}"/>')
        elif state == "live":   # solid lock on bright blue: it really locks
            g = (f'<path d="M9.5 10.5V8.5a2.5 2.5 0 0 1 5 0v2" fill="none" stroke="{GLYPH}" stroke-width="2"/>'
                 f'<rect x="6.5" y="10.5" width="11" height="8" rx="2" fill="{GLYPH}"/>'
                 f'<rect x="11" y="13" width="2" height="3" rx="1" fill="{BLUE_BOT}"/>')
        else:                   # paused (or Bluetooth off): two bars, dimmed tile
            g = (f'<rect x="8.5" y="7.5" width="2.5" height="9" rx="1" fill="{GLYPH_DIM}"/>'
                 f'<rect x="13" y="7.5" width="2.5" height="9" rx="1" fill="{GLYPH_DIM}"/>')
    else:
        phone = (f'<rect x="10.5" y="5" width="7" height="14" rx="1.75" fill="{GLYPH}"/>'
                 f'<rect x="12" y="7" width="4" height="9" rx=".5" fill="{bot}" fill-opacity=".85"/>')
        arrow_col = GLYPH if state == "sending" else ARC_A
        arrow = (f'<path d="M4 12h4.5M6.5 9.5 9 12l-2.5 2.5" fill="none" stroke="{arrow_col}" stroke-width="2" '
                 f'stroke-linecap="round" stroke-linejoin="round"/>')
        g = phone + arrow
        if state == "queued":   # amber badge: something is waiting to send
            g += (f'<circle cx="18" cy="18" r="4.75" fill="{NAVY_BOT}"/>'
                  f'<circle cx="18" cy="18" r="3.5" fill="{AMBER}"/>'
                  f'<path d="M18 16.5V18l1 .75" fill="none" stroke="{NAVY_BOT}" stroke-width="1" stroke-linecap="round"/>')
    return build(24, 1, 6, top, bot, g, edge=False)


TRAY_STATES = {"autolock": ["dry", "live", "paused"], "handoff": ["idle", "sending", "queued"]}


# ------------------------------------------------------------------ render
def render_all():
    sources = {}
    for kind in APPS:
        sources[f"{kind}.svg"] = app_master(kind)
        sources[f"{kind}-24.svg"] = app_24(kind)
        sources[f"{kind}-16.svg"] = app_16(kind)
        for st in TRAY_STATES[kind]:
            sources[f"{kind}-tray-{st}.svg"] = tray(kind, st)
    SRC.mkdir(parents=True, exist_ok=True)
    for name, s in sources.items():
        (SRC / name).write_text(s, encoding="utf-8")

    with sync_playwright() as pw:
        b = pw.chromium.launch()
        page = b.new_page(device_scale_factor=1)

        def raster(name, px):
            page.set_viewport_size({"width": px, "height": px})
            s = sources[name].replace('width="', f'data-w="', 1).replace('height="', 'data-h="', 1)
            page.set_content(f'<html><body style="margin:0;background:transparent">'
                             f'<div style="width:{px}px;height:{px}px">{s.replace("<svg ", f"<svg style=\'display:block;width:{px}px;height:{px}px\' ", 1)}</div></body></html>')
            png = page.screenshot(omit_background=True, clip={"x": 0, "y": 0, "width": px, "height": px})
            return Image.open(io.BytesIO(png)).convert("RGBA")

        for kind, out in APPS.items():
            frames = {16: raster(f"{kind}-16.svg", 16), 20: raster(f"{kind}-24.svg", 20),
                      24: raster(f"{kind}-24.svg", 24)}
            for px in (32, 40, 48, 64, 128, 256):
                frames[px] = raster(f"{kind}.svg", px)
            frames[32].save(out / "32x32.png")
            frames[128].save(out / "128x128.png")
            frames[256].save(out / "icon.png")
            sizes = [16, 20, 24, 32, 40, 48, 64, 256]
            frames[256].save(out / "icon.ico", format="ICO", sizes=[(s, s) for s in sizes],
                             append_images=[frames[s] for s in sizes if s != 256])
            for st in TRAY_STATES[kind]:
                raster(f"{kind}-tray-{st}.svg", 48).save(out / f"tray-{st}.png")
            if kind == "handoff":   # the app loads tray.png today; idle is its resting state
                raster("handoff-tray-idle.svg", 48).save(out / "tray.png")
            # previews for review
            prev = ROOT / "design" / "previews"
            prev.mkdir(exist_ok=True)
        # contact sheet at 1x (and the same at 4x nearest-neighbour to inspect pixels)
        cols = []
        for kind in APPS:
            cols += [raster(f"{kind}-16.svg", 16), raster(f"{kind}-24.svg", 24), raster(f"{kind}.svg", 32),
                     raster(f"{kind}.svg", 256)]
            cols += [raster(f"{kind}-tray-{st}.svg", 24) for st in TRAY_STATES[kind]]
            cols += [raster(f"{kind}-tray-{st}.svg", 48) for st in TRAY_STATES[kind]]
        b.close()

    for bg_name, bg in (("dark", (32, 32, 32, 255)), ("light", (243, 243, 243, 255))):
        W = sum(im.width for im in cols) + 12 * (len(cols) + 1)
        sheet = Image.new("RGBA", (W, 256 + 24), bg)
        x = 12
        for im in cols:
            sheet.alpha_composite(im, (x, 12 + (256 - im.height) // 2)); x += im.width + 12
        sheet.save(ROOT / "design" / "previews" / f"icon-sheet-{bg_name}.png")
    # small sizes blown up 4x with nearest-neighbour, to judge the pixel grid
    small = [im for im in cols if im.width <= 24]
    W = sum(im.width * 4 for im in small) + 8 * (len(small) + 1)
    zs = Image.new("RGBA", (W, 24 * 4 + 16), (32, 32, 32, 255))
    x = 8
    for im in small:
        zs.alpha_composite(im.resize((im.width * 4, im.height * 4), Image.NEAREST), (x, 8)); x += im.width * 4 + 8
    zs.save(ROOT / "design" / "previews" / "icon-pixels-4x.png")
    print("wrote", len(sources), "svg sources and PNG/ICO sets for", ", ".join(APPS))


if __name__ == "__main__":
    render_all()
