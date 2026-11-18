"""Screenshots of every Aura settings page, light and dark, through WebView2's debugging port.

Start the app like this (scratch data, never the live one):
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9333'; Aura.exe --settings
then:  python aura-pc\\tools\\shots.py [port] [outdir]

The Mica backdrop is switched off for the pictures (a page screenshot cannot see it) so colours are the
opaque ones the page falls back to.
"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright

port = sys.argv[1] if len(sys.argv) > 1 else "9333"
out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).resolve().parents[1] / "screenshots"
out.mkdir(exist_ok=True)
PAGES = ["unlock", "lock", "handoff", "devices", "about"]

with sync_playwright() as pw:
    b = pw.chromium.connect_over_cdp(f"http://localhost:{port}")
    page = b.contexts[0].pages[0]
    page.evaluate("document.documentElement.removeAttribute('data-backdrop')")
    for scheme in ("dark", "light"):
        page.emulate_media(color_scheme=scheme)
        for p in PAGES:
            page.evaluate(f"document.querySelector('.nav-item[data-page={p}]').click()")
            page.wait_for_timeout(700)
            page.evaluate("document.querySelector('#main').scrollTop = 0")
            page.screenshot(path=str(out / f"{p}-{scheme}.png"))
            # long pages: second picture scrolled to the bottom
            h = page.evaluate("document.querySelector('#main').scrollHeight - document.querySelector('#main').clientHeight")
            if h > 40:
                page.evaluate("document.querySelector('#main').scrollTop = 1e6")
                page.wait_for_timeout(300)
                page.screenshot(path=str(out / f"{p}-{scheme}-2.png"))
    print("saved to", out)
