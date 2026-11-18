"""Item 17 screenshots: the Away mode card (Devices) and "When you are away" (Unlock), dark and light.
Scratch app only (see shots.py for how to start it):  python aura-pc\\tools\\shots_away.py [port] [outdir]"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "9333"
out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).resolve().parents[1] / "screenshots"
out.mkdir(exist_ok=True)
with sync_playwright() as pw:
    page = pw.chromium.connect_over_cdp(f"http://localhost:{port}").contexts[0].pages[0]
    page.evaluate("document.documentElement.removeAttribute('data-backdrop')")
    for scheme in ("dark", "light"):
        page.emulate_media(color_scheme=scheme)
        for p, anchor, name in (("devices", "#awayH", "away-devices"), ("unlock", "#uAwayH", "away-unlock")):
            page.evaluate(f"document.querySelector('.nav-item[data-page={p}]').click()")
            page.wait_for_timeout(3500)  # one tailscale status call
            page.evaluate(f"document.querySelector('{anchor}').scrollIntoView({{block: 'start'}})")
            page.wait_for_timeout(400)
            page.screenshot(path=str(out / f"{name}-{scheme}.png"))
    print("saved to", out)
