"""Click-test of the Aura settings window (every page, every control) over WebView2's debugging port.

SCRATCH ONLY: start the app with LOCALAPPDATA pointing at a scratch folder, AURA_TEST_HOOKS=1 and
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333, so it never touches the live data.
    python aura-pc\\tools\\qa.py <scratch LOCALAPPDATA> [port]
Prints a PASS/FAIL table and writes qa-results.json next to the scratch folder.
It never triggers a lock (the scratch config is dry run) and never sends anything to the phone.
"""
import json, os, subprocess, sys, time, winreg
from pathlib import Path
from playwright.sync_api import sync_playwright

SCR = Path(sys.argv[1])
PORT = sys.argv[2] if len(sys.argv) > 2 else "9333"
EXE = os.environ.get("AURA_EXE", r"F:\cache\aura-pc-target\debug\aura-pc.exe")
rows = []


def chk(area, name, ok, detail=""):
    rows.append((area, name, "PASS" if ok else "FAIL", str(detail)))
    print(("PASS " if ok else "FAIL ") + f"[{area}] {name} {detail}")


def pend(area, name, why):
    rows.append((area, name, "pending phone" if "phone" in why or "watch" in why else "pending user", why))
    print(f"PEND [{area}] {name}: {why}")


def jread(p):
    try:
        return json.loads(Path(p).read_text(encoding="utf-8"))
    except Exception as e:
        return {"_err": str(e)}


def run_key_value():
    try:
        k = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Run")
        return winreg.QueryValueEx(k, "Aura")[0]
    except OSError:
        return None


def menu(item):
    """Click a tray menu item through the test hook (same function the OS menu calls)."""
    env = dict(os.environ, LOCALAPPDATA=str(SCR), AURA_TEST_HOOKS="1")
    subprocess.run([EXE, "--menu", item], env=env, timeout=20)
    time.sleep(0.8)


def log_text():
    p = SCR / "Aura" / "aura-app.log"
    return p.read_text(encoding="utf-8", errors="replace") if p.exists() else ""


def set_run_value(v):
    k = winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Run", 0, winreg.KEY_SET_VALUE)
    try:
        if v is None:
            try:
                winreg.DeleteValue(k, "Aura")
            except OSError:
                pass
        else:
            winreg.SetValueEx(k, "Aura", 0, winreg.REG_SZ, v)
    finally:
        winreg.CloseKey(k)


# The HKCU Run value "Aura" is shared with the real app: remember it, test from a clean slate, put it back at the end.
ORIG_RUN = run_key_value()
set_run_value(None)
CFG = SCR / "ContinuityAutoLock" / "config.json"
HSET = SCR / "ContinuityHandoff" / "settings.json"
ASET = SCR / "Aura" / "settings.json"
UNL = SCR / "ContinuityUnlock"

# ---- seed scratch files the pages read
UNL.mkdir(parents=True, exist_ok=True)
(UNL / "helper.log").write_text(
    "08:01:10.000Z request aa11: APPROVED in 212 ms\n08:05:00.500Z request bb22: timeout in 1811 ms\n"
    "08:09:30.250Z request cc33: no_device in 8 ms\n08:12:00.000Z request dd44: denied in 950 ms\n", encoding="utf-8")
(UNL / "watch-key.der").write_bytes(b"not a real key")
(UNL / "watch-address.txt").write_text("02:00:00:00:00:01")
(UNL / "pairing.dpapi").write_bytes(b"fake")  # so the Forget phone button is enabled; the scratch app never reads it as a secret in this run

with sync_playwright() as pw:
    b = pw.chromium.connect_over_cdp(f"http://localhost:{PORT}")
    page = b.contexts[0].pages[0]
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    page.reload(); page.wait_for_timeout(2500)
    page.evaluate("document.documentElement.removeAttribute('data-backdrop')")
    inv = lambda cmd, args=None: page.evaluate("([c,a]) => window.__TAURI__.core.invoke(c,a||{})", [cmd, args])
    vis = lambda sel: page.is_visible(sel)

    # ================= navigation
    for i, p in enumerate(["unlock", "lock", "handoff", "remote", "devices", "about"]):
        page.click(f'.nav-item[data-page={p}]')
        page.wait_for_timeout(250)
        ok = vis(f"#page-{p}") and all(not vis(f"#page-{q}") for q in ["unlock", "lock", "handoff", "remote", "devices", "about"] if q != p)
        cur = page.get_attribute(f'.nav-item[data-page={p}]', "aria-current")
        chk("nav", f"sidebar item {p}", ok and cur == "page", f"visible={ok} current={cur}")
    for i, p in enumerate(["unlock", "lock", "handoff", "remote", "devices", "about"]):
        page.keyboard.press(f"Control+{i + 1}")
        page.wait_for_timeout(200)
        chk("nav", f"Ctrl+{i + 1} -> {p}", vis(f"#page-{p}"))

    # ================= UNLOCK
    page.click('.nav-item[data-page=unlock]'); page.wait_for_timeout(1200)
    n = page.locator("#approvals .approval").count()
    t = page.inner_text("#approvals")
    chk("Unlock", "last approvals list (4 seeded lines, newest first)", n == 4 and t.index("Denied") < t.index("Approved"), f"{n} rows")
    chk("Unlock", "approval wording", all(w in t for w in ["Approved", "No answer in time", "No phone or watch reached", "Denied on the phone"]))
    chk("Unlock", "phone and watch rows show names and chips", "the owner" in page.inner_text("#uPhone") and "Watch" in page.inner_text("#uWatch"))
    page.click('label[for=handsFree]'); page.wait_for_timeout(500)
    s = jread(ASET)
    chk("Unlock", "Hands-free switch OFF saved", s.get("handsFree") is False and not page.is_checked("#handsFree"), s)
    chk("Unlock", "hero says hands-free is off", "off" in page.inner_text("#uHeroTitle").lower() or "paired" in page.inner_text("#uHeroTitle").lower(), page.inner_text("#uHeroTitle"))
    chk("Unlock", "helper told (log)", "hands-free sign-in OFF" in log_text())
    page.click('label[for=handsFree]'); page.wait_for_timeout(500)
    chk("Unlock", "Hands-free switch ON saved", jread(ASET).get("handsFree") is True and page.is_checked("#handsFree"), jread(ASET))

    # ---- item 17: When you are away (remote unlock + re-lock). The scratch copy never listens on the tailnet.
    page.wait_for_timeout(2500)  # away_state runs `tailscale status` once per visit
    chk("Away", "Unlock page shows the When you are away section", vis("#remoteUnlock") or page.locator("#remoteUnlock").count() == 1)
    chk("Away", "remote unlock default ON, re-lock default ON at 10 min", page.is_checked("#remoteUnlock") and page.is_checked("#relock") and page.input_value("#relockMin") == "10")
    page.click('label[for=remoteUnlock]'); page.wait_for_timeout(700)
    chk("Away", "Remote unlock switch OFF saved", jread(ASET).get("remoteUnlock") is False and not page.is_checked("#remoteUnlock"), jread(ASET))
    page.click('label[for=remoteUnlock]'); page.wait_for_timeout(700)
    chk("Away", "Remote unlock switch ON saved", jread(ASET).get("remoteUnlock") is True and page.is_checked("#remoteUnlock"))
    chk("Away", "switch logged (applies at once)", "away settings: away_mode=true remote_unlock=true" in log_text())
    page.click('label[for=relock]'); page.wait_for_timeout(700)
    chk("Away", "Re-lock switch OFF saved and minutes disabled", jread(ASET).get("relockAfterRemote") is False and page.is_disabled("#relockMin"))
    page.click('label[for=relock]'); page.wait_for_timeout(700)
    chk("Away", "Re-lock switch ON saved and minutes enabled", jread(ASET).get("relockAfterRemote") is True and not page.is_disabled("#relockMin"))
    page.fill("#relockMin", "999"); page.press("#relockMin", "Tab"); page.wait_for_timeout(700)
    chk("Away", "re-lock minutes clamp to 240", jread(ASET).get("relockMinutes") == 240 and page.input_value("#relockMin") == "240", jread(ASET).get("relockMinutes"))
    page.fill("#relockMin", "15"); page.press("#relockMin", "Tab"); page.wait_for_timeout(700)
    chk("Away", "re-lock minutes 15 saved", jread(ASET).get("relockMinutes") == 15)
    page.fill("#relockMin", "10"); page.press("#relockMin", "Tab"); page.wait_for_timeout(500)
    n = page.locator("#remoteLog .row").count()
    chk("Away", "recent remote actions listed (2 seeded, newest first)", n == 2 and page.inner_text("#remoteLog").index("Unlocked from") < page.inner_text("#remoteLog").index("Lock PC from"), n)
    pend("Away", "remote Unlock PC from the phone (toast + confirmation + re-lock)", "needs the phone away from home (fingerprint)")

    # ================= LOCK
    page.click('.nav-item[data-page=lock]'); page.wait_for_timeout(1200)
    chk("Lock", "hero shows a status", len(page.inner_text("#heroTitle")) > 3, page.inner_text("#heroTitle"))
    chk("Lock", "device list from Windows", page.locator("#devices .row.device").count() >= 2, page.locator("#devices .row.device").count())
    # live switch
    page.click('label[for=liveMode]'); page.wait_for_timeout(700)
    chk("Lock", "Live switch ON -> config live_mode true", jread(CFG).get("live_mode") is True, "scratch config only")
    chk("Lock", "side chip says Live", page.inner_text("#sideMode") == "Live")
    page.click('label[for=liveMode]'); page.wait_for_timeout(700)
    chk("Lock", "Live switch OFF -> dry run", jread(CFG).get("live_mode") is False and page.inner_text("#sideMode") == "Dry run")
    # pause
    page.click("#pauseBtn"); page.wait_for_timeout(2500)
    chk("Lock", "Pause button pauses", "Resume" in page.inner_text("#pauseBtn") and "Back on in" in page.inner_text("#pauseD"), page.inner_text("#pauseD"))
    page.click("#pauseBtn"); page.wait_for_timeout(2500)
    chk("Lock", "Resume button resumes", "Pause" in page.inner_text("#pauseBtn") and "Back on in" not in page.inner_text("#pauseD"), page.inner_text("#pauseD"))
    # form
    chk("Lock", "Save disabled until a change", page.is_disabled("#save"))
    before = jread(CFG)
    page.fill("#absent", "45"); chk("Lock", "typing enables Save", not page.is_disabled("#save"))
    page.click("#revert"); page.wait_for_timeout(200)
    chk("Lock", "Undo changes restores the form", page.input_value("#absent") == str(before["absent_secs"]) and page.is_disabled("#save"))
    page.fill("#absent", "999"); page.press("#absent", "Tab")
    chk("Lock", "away seconds clamps to 600", page.input_value("#absent") == "600", page.input_value("#absent"))
    page.fill("#absent", "45")
    page.click('label:has(input[value=any_absent])')
    page.click('label[for=userssi]'); page.click('label[for=fullscreen]')
    page.fill("#ping", "30"); page.fill("#idleGuard", "90"); page.fill("#minMisses", "7"); page.fill("#singleSignal", "75")
    page.fill("#offwristSecs", "9"); page.click('label[for=offwrist]'); page.click('label[for=linkSignals]')
    page.fill("#rssi", "-80") if False else page.evaluate("(()=>{const r=document.getElementById('rssi');r.value=-80;r.dispatchEvent(new Event('input',{bubbles:true}))})()")
    # device checkbox: toggle the first unchecked one on, then off again
    first = page.locator("#devices .row.device input.check").first
    first.click(); first.click()
    page.click("#save"); page.wait_for_timeout(800)
    c = jread(CFG)
    want = dict(absent_secs=45, lock_when="any_absent", use_rssi=not before["use_rssi"], block_on_fullscreen=not before["block_on_fullscreen"],
                ping_interval_secs=30, idle_guard_secs=90, min_misses=7, single_signal_secs=75, offwrist_hint=False, offwrist_lock_secs=9, link_signals=True, rssi_threshold=-80)
    bad = {k: (c.get(k), v) for k, v in want.items() if c.get(k) != v}
    chk("Lock", "Save writes every setting to config.json", not bad, bad or "all 12 fields")
    chk("Lock", "live_mode untouched by Save", c.get("live_mode") is False)
    chk("Lock", "devices kept", [d["name"] for d in c["devices"]] == [d["name"] for d in before["devices"]], [d["name"] for d in c["devices"]])
    chk("Lock", "Saved note + Save disabled again", page.is_disabled("#save") and "Saved" in page.inner_text("#saved"))
    page.keyboard.press("Control+s")  # nothing dirty: must not error
    # restore original values (still the scratch config)
    page.fill("#absent", "20"); page.click('label:has(input[value=all_absent])')
    page.click('label[for=userssi]'); page.click('label[for=fullscreen]')
    page.fill("#ping", "10"); page.fill("#idleGuard", "60"); page.fill("#minMisses", "5"); page.fill("#singleSignal", "60")
    page.click('label[for=offwrist]'); page.fill("#offwristSecs", "5"); page.click('label[for=linkSignals]')
    page.evaluate("(()=>{const r=document.getElementById('rssi');r.value=-90;r.dispatchEvent(new Event('input',{bubbles:true}))})()")
    page.keyboard.press("Control+s"); page.wait_for_timeout(800)
    c = jread(CFG)
    chk("Lock", "Ctrl+S saves", c.get("absent_secs") == 20 and c.get("link_signals") is False and c.get("rssi_threshold") == -90, c)
    chk("Lock", "rssi row disabled follows its switch", True if page.is_checked("#userssi") else page.is_disabled("#rssi"))
    page.evaluate("(()=>{const e=document.getElementById('userssi'); if(e.checked) e.click();})()")  # switch off -> row disabled
    chk("Lock", "signal slider disabled when the switch is off", page.is_disabled("#rssi"))
    page.click("#revert") if not page.is_hidden("#revert") else None

    # ================= HANDOFF
    page.click('.nav-item[data-page=handoff]'); page.wait_for_timeout(1500)
    chk("Handoff", "hero shows phone state", len(page.inner_text("#hPeer")) > 3, page.inner_text("#hPeer"))
    chk("Handoff", "hotkey keycaps Ctrl+Alt+H", page.inner_text("#hotkey").replace("\n", "") .replace(" ", "") in ("Ctrl+Alt+H", "Ctrl+Alt+H"), page.inner_text("#hotkey"))
    chk("Handoff", "snip keycaps Ctrl+Alt+S", "S" in page.inner_text("#snipKey"))
    page.click("#hotkey"); page.keyboard.press("Control+Alt+J"); page.wait_for_timeout(700)
    chk("Handoff", "record a new send shortcut -> saved", jread(HSET).get("hotkey") == "Ctrl+Alt+J", jread(HSET).get("hotkey"))
    chk("Handoff", "...and registered", "hotkey registered: Ctrl+Alt+J" in log_text())
    page.click("#hotkey"); page.keyboard.press("Control+Alt+S"); page.wait_for_timeout(700)
    # Ctrl+Alt+S is the snip shortcut of this same copy -> must show the "taken" error and keep the old key
    chk("Handoff", "a taken shortcut shows an error and keeps the old one", (not page.is_hidden("#hotkeyError")) and jread(HSET).get("hotkey") == "Ctrl+Alt+J", page.inner_text("#hotkeyError"))
    page.click("#hotkey"); page.keyboard.press("Shift+K"); page.wait_for_timeout(300)
    chk("Handoff", "a shortcut with no Ctrl/Alt/Win is refused", "Add Ctrl" in page.inner_text("#hotkey"), page.inner_text("#hotkey"))
    page.keyboard.press("Escape"); page.wait_for_timeout(200)
    chk("Handoff", "Esc cancels recording", "recording" not in (page.get_attribute("#hotkey", "class") or ""))
    page.click("#snipKey"); page.keyboard.press("Control+Alt+Y"); page.wait_for_timeout(600)
    chk("Handoff", "record a new snip shortcut", jread(HSET).get("snipHotkey") == "Ctrl+Alt+Y", jread(HSET).get("snipHotkey"))
    page.click("#snipKey"); page.keyboard.press("Backspace"); page.wait_for_timeout(600)
    chk("Handoff", "Backspace turns the snip shortcut off", jread(HSET).get("snipHotkey") == "" and "Off" in page.inner_text("#snipKey"), page.inner_text("#snipKey"))
    for sid, key in [("youtubeTimestamp", "youtubeTimestamp"), ("clipboardSync", "clipboardSync"), ("suggest", "suggest"), ("openInBrave", "openInBrave")]:
        was = jread(HSET).get(key, True)
        page.click(f'label[for={sid}]'); page.wait_for_timeout(500)
        chk("Handoff", f"switch {sid}", jread(HSET).get(key) == (not was), f"{was} -> {jread(HSET).get(key)}")
        page.click(f'label[for={sid}]'); page.wait_for_timeout(500)
    # put the shortcuts back
    page.click("#hotkey"); page.keyboard.press("Control+Alt+K"); page.wait_for_timeout(500)
    page.click("#snipKey"); page.keyboard.press("Control+Alt+L"); page.wait_for_timeout(500)
    chk("Handoff", "inbox path shown", "Downloads" in page.inner_text("#inboxPath"), page.inner_text("#inboxPath"))
    page.click("#openInbox"); page.wait_for_timeout(500)
    chk("Handoff", "Open folder (Downloads\\Aura) runs without error", True)
    r = inv("send_paths", {"paths": [str(SCR)]})  # a folder: must be refused, nothing is sent
    chk("Handoff", "drop zone accepts only files (a folder sends 0)", r == 0, r)
    pend("Handoff", "drop a real file on the drop zone", "needs a real OS drag and a reachable phone")
    pend("Handoff", "Ctrl+Alt+H / Ctrl+Alt+S delivery", "needs the phone (hotkey press -> phone)")

    # ================= DEVICES
    page.click('.nav-item[data-page=devices]'); page.wait_for_timeout(1500)
    chk("Devices", "watch pairing seeded -> Forget watch enabled", not page.is_disabled('[data-act=forget-watch]'))
    page.click('[data-act=forget-watch]'); page.wait_for_timeout(300)
    chk("Devices", "Forget watch asks first", page.is_visible("#dlg") and "Forget the watch" in page.inner_text("#dlgTitle"))
    page.click("#dlgCancel"); page.wait_for_timeout(300)
    chk("Devices", "Cancel keeps the files", (UNL / "watch-key.der").exists() and not page.is_visible("#dlg"))
    page.click('[data-act=forget-watch]'); page.click("#dlgOk"); page.wait_for_timeout(900)
    moved = list(UNL.glob("forgotten-*/watch-key.der"))
    chk("Devices", "Forget watch moves the files to a backup", (not (UNL / "watch-key.der").exists()) and len(moved) == 1, [str(m.parent.name) for m in moved])
    chk("Devices", "restart banner appears", not page.is_hidden("#restartBanner"))
    page.click("#dlgCancel"); page.wait_for_timeout(300)
    page.click('[data-act=pair-watch]'); page.wait_for_timeout(300)
    chk("Devices", "Pair watch explains the 60 s window", page.is_visible("#dlg") and "60 seconds" in page.inner_text("#dlgText"))
    page.click("#dlgOk"); page.wait_for_timeout(8000)
    chk("Devices", "Pair watch with no watch in reach ends in a clear error", page.is_visible("#dlg") and "Watch not paired" in page.inner_text("#dlgTitle"), page.inner_text("#dlgText")[:120])
    page.click("#dlgCancel"); page.wait_for_timeout(300)
    pend("Devices", "Pair watch success (fingerprint, pin)", "needs the watch in its pairing window")
    page.click('[data-act=repair-phone]'); page.wait_for_timeout(200)
    chk("Devices", "Re-pair asks first", page.is_visible("#dlg") and "Re-pair" in page.inner_text("#dlgTitle"))
    page.click("#dlgOk"); page.wait_for_timeout(1200)
    code = page.inner_text("#dlgCode")
    chk("Devices", "Re-pair shows a new pairing code (scratch)", page.is_visible("#dlgCode") and len("".join(code.split()).replace("-", "")) >= 34, code)
    page.click("#dlgCancel"); page.wait_for_timeout(300)
    chk("Devices", "old phone pairing backed up", len(list(UNL.glob("forgotten-*/pairing.dpapi"))) >= 1)
    pend("Devices", "enter the new code in the phone app and use it", "needs the phone")
    page.click('.nav-item[data-page=lock]'); page.click('.nav-item[data-page=devices]'); page.wait_for_timeout(1500)  # reload the state: the new scratch pairing exists now
    chk("Devices", "Forget phone enabled once a (scratch) pairing exists", not page.is_disabled('[data-act=forget-phone]'))
    page.click('[data-act=forget-phone]'); page.wait_for_timeout(300)
    chk("Devices", "Forget phone asks first", page.is_visible("#dlg") and "Forget the phone" in page.inner_text("#dlgTitle"))
    page.click("#dlgCancel"); page.wait_for_timeout(300)
    chk("Devices", "Cancel keeps the phone pairing", (UNL / "pairing.dpapi").exists())
    page.click('[data-act=forget-phone]'); page.click("#dlgOk"); page.wait_for_timeout(900)
    chk("Devices", "Forget phone moves the pairing to a backup", not (UNL / "pairing.dpapi").exists() and len(list(UNL.glob("forgotten-*/pairing.dpapi"))) >= 2)
    page.click("#dlgCancel"); page.wait_for_timeout(300)

    # ---- item 17: Away mode card (real, read-only `tailscale status --json`)
    page.click('.nav-item[data-page=lock]'); page.click('.nav-item[data-page=devices]'); page.wait_for_timeout(3500)
    code = page.get_attribute("#dAway", "data-code")
    chk("Away", "Away card shows a Tailscale state", code in ["ready", "missing", "off", "login", "no-phone", "phone-offline", "starting", "unknown"], f"{code}: {page.inner_text('#awayTitle')}")
    chk("Away", "Away card chip matches the state", len(page.inner_text("#awayChip")) > 1, page.inner_text("#awayChip"))
    pc = page.inner_text("#awayPc")
    chk("Away", "this PC's tailnet address shown", pc.startswith("100."), pc)
    chk("Away", "the scratch copy is not listening on the tailnet", "listening" not in pc, pc)
    chk("Away", "phone address row shown", len(page.inner_text("#awayPhone")) > 3, page.inner_text("#awayPhone"))
    chk("Away", "action button hidden when ready / shown otherwise", page.is_hidden("#awayAction") == (code in ["ready", "no-phone", "phone-offline", "starting"]), page.inner_text("#awayAction") if not page.is_hidden("#awayAction") else "hidden")
    chk("Away", "winget button only when Tailscale is missing", page.is_hidden("#awayWinget") == (code != "missing"))
    page.click("#awayRefresh"); page.wait_for_timeout(300)
    busy = page.is_disabled("#awayRefresh")
    page.wait_for_timeout(3000)
    chk("Away", "Check again re-reads Tailscale (button busy, then back)", not page.is_disabled("#awayRefresh"), f"busy during call={busy}")
    page.evaluate("document.getElementById('restartBanner').hidden = true")
    page.click('label[for=awayMode]'); page.wait_for_timeout(2500)
    chk("Away", "Away mode switch OFF saved", jread(ASET).get("awayMode") is False and not page.is_checked("#awayMode"), jread(ASET))
    chk("Away", "Away mode OFF -> card says off + restart banner", page.get_attribute("#dAway", "data-code") == "disabled" and not page.is_hidden("#restartBanner"))
    page.click('label[for=awayMode]'); page.wait_for_timeout(2500)
    chk("Away", "Away mode switch ON saved", jread(ASET).get("awayMode") is True and page.is_checked("#awayMode"))
    try:
        inv("tailscale_action", {"which": "bogus"}); bad = False
    except Exception as e:
        bad = "unknown action" in str(e)
    chk("Away", "unknown Tailscale action is refused", bad)
    pend("Away", "Get Tailscale / Open Tailscale / Copy winget buttons", "user: they open the browser / the Tailscale app (not clicked by the test)")

    # ================= ABOUT
    page.click('.nav-item[data-page=about]'); page.wait_for_timeout(800)
    chk("About", "version shown", "1.0.0" in page.inner_text("#aboutVer"), page.inner_text("#aboutVer"))
    before_run = run_key_value()
    page.click('label[for=autostart]'); page.wait_for_timeout(600)
    v = run_key_value()
    chk("About", "Start with Windows ON writes HKCU Run 'Aura'", bool(v) and "aura-pc.exe" in v.lower() and "--autostart" in v, v)
    page.click('label[for=autostart]'); page.wait_for_timeout(600)
    chk("About", "Start with Windows OFF removes it", run_key_value() is None, run_key_value())
    page.click("#openData"); page.wait_for_timeout(400)
    chk("About", "Open folder (logs and settings)", True)
    page.click("#restartBtn"); page.wait_for_timeout(300)
    chk("About", "Restart asks first", page.is_visible("#dlg") and "Restart Aura" in page.inner_text("#dlgTitle"))
    page.click("#dlgCancel"); page.wait_for_timeout(300)
    page.click("#exitBtn"); page.wait_for_timeout(300)
    chk("About", "Exit asks first", page.is_visible("#dlg") and "Exit Aura" in page.inner_text("#dlgTitle"))
    page.click("#dlgCancel"); page.wait_for_timeout(300)

    # ================= TRAY (through the same function the OS menu calls)
    n0 = log_text().count("hands-free sign-in")
    menu("handsfree")
    chk("Tray", "Hands-free sign-in item turns it OFF", jread(ASET).get("handsFree") is False)
    menu("handsfree")
    chk("Tray", "...and ON again", jread(ASET).get("handsFree") is True and log_text().count("hands-free sign-in") >= n0 + 2)
    menu("live")
    chk("Tray", "Auto-lock is live item turns live ON (scratch config)", jread(CFG).get("live_mode") is True and "mode switched to LIVE" in log_text(), jread(CFG).get("live_mode"))
    menu("live")
    chk("Tray", "...and back to dry run", jread(CFG).get("live_mode") is False)
    menu("pause"); page.wait_for_timeout(1500)
    chk("Tray", "Pause item pauses", "paused for 60 min" in log_text())
    menu("pause"); page.wait_for_timeout(1500)
    chk("Tray", "Pause item (again) resumes", log_text().count("resumed") >= 1)
    menu("autostart")
    chk("Tray", "Start with Windows item writes HKCU Run", run_key_value() is not None, run_key_value())
    menu("autostart")
    chk("Tray", "...and removes it", run_key_value() is None)
    menu("settings")
    chk("Tray", "Settings item opens the window", page.is_visible("#main"))
    menu("logs")
    chk("Tray", "Open log folder item", True)
    menu("q:retry"); chk("Tray", "Waiting to send > Retry now (empty queue, no error)", "panicked" not in log_text())
    menu("q:clear"); chk("Tray", "Waiting to send > Clear queue", "panicked" not in log_text())
    menu("inbox_clear"); chk("Tray", "From phone > Clear list", "panicked" not in log_text())
    pend("Tray", "Snip to phone / Send what I'm looking at / Send a file...", "needs an interactive desktop (the PC session is locked right now) and the phone")
    pend("Tray", "Continue on PC suggestion, From phone items", "needs the phone to send something")

    # ================= no JS errors
    chk("Page", "no JavaScript errors during the run", not errors, errors[:3])

    # ================= restart and exit (last: they end the scratch process)
    page.click('.nav-item[data-page=about]')
    def pids():
        out = subprocess.run(["powershell", "-NoProfile", "-Command", "(Get-Process aura-pc -ErrorAction SilentlyContinue | ? { $_.Path -ieq '" + EXE + "' }).Id -join ','"], capture_output=True, text=True).stdout.strip()
        return out
    p0 = pids()
    page.click("#restartBtn"); page.click("#dlgOk")
    time.sleep(9)
    p1 = pids()
    chk("About", "Restart Aura starts a new process and ends the old one", bool(p1) and p1 != p0, f"{p0} -> {p1}")
    menu("settings"); time.sleep(4)
    try:
        import urllib.request
        up = b"Aura" in urllib.request.urlopen(f"http://localhost:{PORT}/json", timeout=5).read()
    except Exception as e:
        up = False
    chk("About", "restarted Aura serves the window again", up)
    if up:
        b2 = pw.chromium.connect_over_cdp(f"http://localhost:{PORT}")
        pg = b2.contexts[0].pages[0]
        pg.click('.nav-item[data-page=about]'); pg.click("#exitBtn"); pg.click("#dlgOk")
        time.sleep(4)
        chk("About", "Exit Aura ends the process", pids() == "", pids())

    set_run_value(ORIG_RUN)
    out = SCR.parent / "aura-qa-results.json"
    out.write_text(json.dumps(rows, indent=1), encoding="utf-8")
    print("\nPASS", sum(r[2] == "PASS" for r in rows), "FAIL", sum(r[2] == "FAIL" for r in rows), "pending", sum(r[2].startswith("pending") for r in rows))
