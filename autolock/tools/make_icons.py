"""Draws the app icon (rounded blue tile + white padlock). Run once: python tools/make_icons.py"""
from PIL import Image, ImageDraw
import os
out = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")
def draw(size, accent=(0, 103, 192)):
    s = 1024
    im = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([40, 40, s - 40, s - 40], radius=220, fill=accent + (255,))
    w = (255, 255, 255, 255)
    d.rounded_rectangle([250, 470, 774, 860], radius=70, fill=w)        # body
    d.arc([330, 170, 694, 560], start=180, end=360, fill=w, width=95)   # shackle top
    d.rectangle([330, 360, 378, 480], fill=w); d.rectangle([646, 360, 694, 480], fill=w)
    d.ellipse([462, 590, 562, 690], fill=accent + (255,)); d.rectangle([494, 640, 530, 760], fill=accent + (255,))
    return im.resize((size, size), Image.LANCZOS)
big = draw(256)
big.save(os.path.join(out, "icon.ico"), sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (48, 48), (64, 64), (256, 256)])
draw(32).save(os.path.join(out, "32x32.png")); draw(128).save(os.path.join(out, "128x128.png")); draw(256).save(os.path.join(out, "icon.png"))
draw(64, (196, 43, 28)).save(os.path.join(out, "tray-live.png"))     # red tile = live mode
draw(64, (0, 103, 192)).save(os.path.join(out, "tray-dry.png"))      # blue tile = dry run
draw(64, (110, 110, 110)).save(os.path.join(out, "tray-paused.png")) # grey = paused / off
print("ok")
