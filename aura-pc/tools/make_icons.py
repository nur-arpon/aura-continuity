"""Aura PC icon builder: the app icon (from design\\aura-icon\\aura-icon.svg) and the five tray states.

    python aura-pc\\tools\\make_icons.py

Tray states (48 px PNG drawn on a 24 grid, same convention as design\\tools\\make_icons.py):
    idle       nothing nearby / paused / not paired      slate tile, dim rings
    connected  a phone or watch is nearby or linked      navy tile, bright rings, teal badge
    sending    Handoff is sending something              blue tile, rings + outbound arrow
    locked     the PC session is locked / locking        navy tile, padlock
    error      Bluetooth off or the Aura service failed  navy tile, dim rings, amber "!" badge
Shape differs per state, not only colour. Rendered by Playwright's Chromium (transparent PNGs).
"""
import io
from pathlib import Path
from PIL import Image
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "aura-pc" / "src-tauri" / "icons"
UI = ROOT / "aura-pc" / "ui"
SRC = ROOT / "design" / "aura-icon" / "aura-icon.svg"

NAVY_TOP, NAVY_BOT = "#1F4C8F", "#0B1B38"
BLUE_TOP, BLUE_BOT = "#4A98FF", "#1D5FD0"
SLATE_TOP, SLATE_BOT = "#33415C", "#18223A"
GLYPH, GLYPH_DIM = "#EEF4FF", "#9FB2D1"
SKY, TEAL, AMBER, INK = "#8EC5FF", "#4CD3B4", "#F3B95F", "#0B1B38"


def svg(view, body, defs=""):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view} {view}" width="{view}" height="{view}">'
            f"<defs>{defs}</defs>{body}</svg>")


def tile24(top, bot, glyph):
    d = (f'<linearGradient id="t" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{top}"/>'
         f'<stop offset="1" stop-color="{bot}"/></linearGradient>')
    return svg(24, f'<rect x="1" y="1" width="22" height="22" rx="6" fill="url(#t)"/>' + glyph, d)


def rings(c_out, c_in, dot, w_out=1.25, w_in=1.75, dot_r=2.25, cx=12, cy=12):
    return (f'<circle cx="{cx}" cy="{cy}" r="8.25" fill="none" stroke="{c_out}" stroke-width="{w_out}" stroke-opacity=".6"/>'
            f'<circle cx="{cx}" cy="{cy}" r="5" fill="none" stroke="{c_in}" stroke-width="{w_in}"/>'
            f'<circle cx="{cx}" cy="{cy}" r="{dot_r}" fill="{dot}"/>')


def tray_svgs():
    s = {}
    s["idle"] = tile24(SLATE_TOP, SLATE_BOT, rings(GLYPH_DIM, GLYPH_DIM, GLYPH_DIM))
    s["connected"] = tile24(NAVY_TOP, NAVY_BOT,
                            rings(SKY, SKY, "#FFFFFF")
                            + f'<circle cx="18.25" cy="18.25" r="4.4" fill="{NAVY_BOT}"/><circle cx="18.25" cy="18.25" r="3.1" fill="{TEAL}"/>')
    s["sending"] = tile24(BLUE_TOP, BLUE_BOT,
                          '<circle cx="8" cy="12" r="4.75" fill="none" stroke="#FFFFFF" stroke-width="1.75"/><circle cx="8" cy="12" r="2" fill="#FFFFFF"/>'
                          '<path d="M14.75 12h5.5M17.5 8.75 20.75 12 17.5 15.25" fill="none" stroke="#FFFFFF" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>')
    s["locked"] = tile24(NAVY_TOP, NAVY_BOT,
                         f'<path d="M9.25 10.75V8.75a2.75 2.75 0 0 1 5.5 0v2" fill="none" stroke="{GLYPH}" stroke-width="2"/>'
                         f'<rect x="6.75" y="10.75" width="10.5" height="8" rx="2" fill="{GLYPH}"/>'
                         f'<circle cx="12" cy="14.25" r="1.3" fill="{NAVY_BOT}"/><rect x="11.35" y="14.5" width="1.3" height="2.4" rx=".65" fill="{NAVY_BOT}"/>')
    s["error"] = tile24(NAVY_TOP, NAVY_BOT,
                        rings(GLYPH_DIM, GLYPH_DIM, GLYPH_DIM, cx=11, cy=11)
                        + f'<circle cx="17.75" cy="17.75" r="5.1" fill="{NAVY_BOT}"/><circle cx="17.75" cy="17.75" r="4" fill="{AMBER}"/>'
                        f'<path d="M17.75 15.2v3" stroke="{NAVY_BOT}" stroke-width="1.5" stroke-linecap="round"/><circle cx="17.75" cy="20" r=".85" fill="{NAVY_BOT}"/>')
    return s


def app_master():
    """aura-icon.svg is a full-bleed 108 square (adaptive icon). Windows wants the rounded tile."""
    inner = SRC.read_text(encoding="utf-8")
    body = inner[inner.index("<defs>"):inner.rindex("</svg>")]
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 108 108" width="108" height="108">'
            '<clipPath id="r"><rect x="3" y="3" width="102" height="102" rx="26"/></clipPath>'
            f'<g clip-path="url(#r)">{body.replace("<defs>", "<defs>", 1)}</g></svg>')


def app_small(px):
    """16/24 px: one ring and a dot on a plain navy tile (the field gradient would be mud)."""
    r, w, dr = (6.1, 1.6, 2.4) if px == 24 else (4.6, 1.3, 1.9)
    c = px / 2
    d = ('<linearGradient id="t" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1F4C8F"/>'
         '<stop offset="1" stop-color="#0B1B38"/></linearGradient>')
    k = 6 if px == 24 else 4
    body = (f'<rect x="{0.5 if px==24 else 0}" y="{0.5 if px==24 else 0}" width="{px-(1 if px==24 else 0)}" height="{px-(1 if px==24 else 0)}" rx="{k}" fill="url(#t)"/>'
            f'<circle cx="{c}" cy="{c}" r="{r}" fill="none" stroke="{SKY}" stroke-width="{w}"/>'
            f'<circle cx="{c}" cy="{c}" r="{dr}" fill="#FFFFFF"/>')
    return svg(px, body, d)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    UI.mkdir(parents=True, exist_ok=True)
    tr = tray_svgs()
    master = app_master()
    (UI / "app-mark.svg").write_text(master, encoding="utf-8")
    with sync_playwright() as pw:
        b = pw.chromium.launch()
        page = b.new_page(device_scale_factor=1)

        def raster(s, px):
            page.set_viewport_size({"width": px, "height": px})
            s = s.replace("<svg ", f"<svg style='display:block;width:{px}px;height:{px}px' ", 1)
            page.set_content(f'<html><body style="margin:0;background:transparent">{s}</body></html>')
            png = page.screenshot(omit_background=True, clip={"x": 0, "y": 0, "width": px, "height": px})
            return Image.open(io.BytesIO(png)).convert("RGBA")

        frames = {16: raster(app_small(16), 16), 20: raster(app_small(24), 20), 24: raster(app_small(24), 24)}
        for px in (32, 40, 48, 64, 128, 256):
            frames[px] = raster(master, px)
        frames[32].save(OUT / "32x32.png")
        frames[128].save(OUT / "128x128.png")
        frames[256].save(OUT / "icon.png")
        sizes = [16, 20, 24, 32, 40, 48, 64, 256]
        frames[256].save(OUT / "icon.ico", format="ICO", sizes=[(s, s) for s in sizes],
                         append_images=[frames[s] for s in sizes if s != 256])
        sheet = []
        for name, s in tr.items():
            (OUT / f"tray-{name}.svg").write_text(s, encoding="utf-8")
            raster(s, 48).save(OUT / f"tray-{name}.png")
            sheet += [raster(s, 24), raster(s, 48)]
        b.close()
    prev = ROOT / "aura-pc" / "screenshots"
    prev.mkdir(exist_ok=True)
    for bg_name, bg in (("dark", (32, 32, 32, 255)), ("light", (243, 243, 243, 255))):
        W = sum(im.width for im in sheet) + 14 * (len(sheet) + 1) + 280
        img = Image.new("RGBA", (W, 280), bg)
        x = 14
        img.alpha_composite(frames[256], (x, 12)); x += 270
        for im in sheet:
            img.alpha_composite(im, (x, 120 + (48 - im.height) // 2)); x += im.width + 14
        img.save(prev / f"icons-{bg_name}.png")
    print("icons written to", OUT)


if __name__ == "__main__":
    main()
