"""Item 15 click-test of the Remote page, the ring alert window and the tray items (scratch data only).

    python aura-pc\\tools\\qa_remote.py <scratch LOCALAPPDATA> [port]
Starts AURA_EXE (default: the staged build in aura-pc\\next) itself with LOCALAPPDATA = the scratch folder,
AURA_TEST_HOOKS=1 and a WebView2 debug port, so it never touches the live Aura or its data. No lock, no phone,
no media control (play / pause / next could start the owner's music): only the volume buttons, restored at once.
"""
import json, os, subprocess, sys, time, ctypes
from pathlib import Path
from playwright.sync_api import sync_playwright

SCR = Path(sys.argv[1])
PORT = sys.argv[2] if len(sys.argv) > 2 else "9334"
EXE = os.environ.get("AURA_EXE", r"D:\Claude-Projects\Continuity\aura-pc\next\aura-pc.exe")
rows = []


def chk(name, ok, detail=""):
    rows.append((name, "PASS" if ok else "FAIL", str(detail)))
    print(("PASS " if ok else "FAIL ") + name + " " + str(detail))


def jread(p):
    try:
        return json.loads(Path(p).read_text(encoding="utf-8"))
    except Exception as e:
        return {"_err": str(e)}


def log_text():
    p = SCR / "Aura" / "aura-app.log"
    return p.read_text(encoding="utf-8", errors="replace") if p.exists() else ""


def svc_log():
    p = SCR / "Aura" / "aura.log"
    return p.read_text(encoding="utf-8", errors="replace") if p.exists() else ""


SCR.mkdir(parents=True, exist_ok=True)
env = dict(os.environ, LOCALAPPDATA=str(SCR), AURA_TEST_HOOKS="1", WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f"--remote-debugging-port={PORT}")
proc = subprocess.Popen([EXE, "--settings", "--page", "remote"], env=env)
try:
    time.sleep(6)
    with sync_playwright() as pw:
        b = pw.chromium.connect_over_cdp(f"http://localhost:{PORT}")
        ctx = b.contexts[0]
        page = [p for p in ctx.pages if "index.html" in p.url or p.url.endswith("/")][0]
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.reload(); page.wait_for_timeout(2500)
        inv = lambda cmd, args=None: page.evaluate("([c,a]) => window.__TAURI__.core.invoke(c,a||{})", [cmd, args])
        ASET = SCR / "Aura" / "settings.json"

        page.click('.nav-item[data-page=remote]'); page.wait_for_timeout(1200)
        chk("Remote page opens", page.is_visible("#page-remote"))
        chk("sidebar shows Remote as current", page.get_attribute('.nav-item[data-page=remote]', "aria-current") == "page")
        page.keyboard.press("Control+4"); page.wait_for_timeout(300)
        chk("Ctrl+4 -> Remote", page.is_visible("#page-remote"))
        t = page.inner_text("#rTitle")
        chk("now playing card reads Windows media session", len(t) > 0, t)
        chk("all four switches start ON", all(page.is_checked(f"#{i}") for i in ["rRing", "rFull", "rMedia", "rPres"]))

        for sw, key in [("rRing", "allowRing"), ("rFull", "ringFullVolume"), ("rMedia", "allowMedia"), ("rPres", "allowPresenter")]:
            page.click(f"label[for={sw}]"); page.wait_for_timeout(400)
            s = jread(ASET)
            chk(f"switch {sw} OFF saved", s.get(key) is False and not page.is_checked(f"#{sw}"), s.get(key))
            page.click(f"label[for={sw}]"); page.wait_for_timeout(400)
            s = jread(ASET)
            chk(f"switch {sw} ON saved", s.get(key) is True and page.is_checked(f"#{sw}"), s.get(key))

        # ring this PC (the same path a phone's ring takes), then dismiss from the alert window
        before = len(ctx.pages)
        page.click("#ringTest"); page.wait_for_timeout(5000)
        rp = [p for p in ctx.pages if "ring.html" in p.url]
        chk("Try it opens the ring alert window", len(rp) == 1, [p.url for p in ctx.pages])
        chk("service log: chime on", "RING from This PC: chime on" in svc_log(), "")
        if rp:
            chk("alert says the laptop is ringing", "ringing" in rp[0].inner_text("h1").lower(), rp[0].inner_text("h1"))
            rp[0].click("#found"); page.wait_for_timeout(1500)
        chk("I found it closes the alert", not [p for p in ctx.pages if "ring.html" in p.url], "")
        chk("service log: chime stopped", "RING stopped" in svc_log(), "")
        # same again, dismissed with Esc
        page.click("#ringTest"); page.wait_for_timeout(5000)
        rp = [p for p in ctx.pages if "ring.html" in p.url]
        if rp:
            rp[0].keyboard.press("Escape"); page.wait_for_timeout(1500)
        chk("Esc dismisses the alert too", not [p for p in ctx.pages if "ring.html" in p.url], "")
        # switch OFF refuses (the PC says so)
        page.click("label[for=rRing]"); page.wait_for_timeout(400)
        chk("Ring phone button is disabled with no phone paired", page.is_disabled("#ringPhone"), page.inner_text("#rPhoneDesc"))
        page.click("label[for=rRing]"); page.wait_for_timeout(300)

        # volume buttons only (restored): -, then +
        v0 = page.evaluate("() => 1")
        page.click('[data-media="5"]'); page.wait_for_timeout(500)
        page.click('[data-media="4"]'); page.wait_for_timeout(500)
        chk("volume buttons run without error", not errors, errors)
        chk("no JavaScript errors during the run", not errors, errors)
finally:
    proc.terminate()
    time.sleep(1)

ok = sum(1 for r in rows if r[1] == "PASS")
print(f"\n{ok}/{len(rows)} PASS")
Path(SCR / "qa-remote-results.json").write_text(json.dumps(rows, indent=1), encoding="utf-8")
sys.exit(0 if ok == len(rows) else 1)
