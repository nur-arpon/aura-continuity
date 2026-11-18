// Aura settings window. Plain JS, Tauri global API (withGlobalTauri). One window, five pages.
// Backends: aura_state & co (aura-pc), get_config/get_status/list_paired/save_config (auto-lock),
// get_state/save_settings/send_paths (Handoff). Events "status" and "config" come from the auto-lock.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const win = window.__TAURI__.window.getCurrentWindow();

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
function icon(name, cls = "ic") {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("class", cls); s.setAttribute("aria-hidden", "true");
  const u = document.createElementNS("http://www.w3.org/2000/svg", "use");
  u.setAttribute("href", `#i-${name}`); s.append(u); return s;
}
function chip(text, tone) { const c = el("span", "chip", text); if (tone) c.dataset.tone = tone; return c; }
function list(names) {
  if (names.length <= 1) return names.join("");
  return names.slice(0, -1).join(", ") + " and " + names.at(-1);
}
const secs = (ms) => Math.floor(ms / 1000);
const timeouts = {};
function note(id, text, tone, sticky) {
  const n = $(id);
  clearTimeout(timeouts[id]);
  n.textContent = text; n.dataset.tone = tone || "";
  n.classList.toggle("show", !!text);
  if (!sticky && text) timeouts[id] = setTimeout(() => n.classList.remove("show"), 2400);
}

// ------------------------------------------------------------------ navigation
const PAGES = ["unlock", "lock", "handoff", "remote", "devices", "about", "playing"];
let page = null;
function go(name, push = true) {
  if (!PAGES.includes(name)) name = "unlock";
  if (page === name) return;
  page = name;
  for (const p of PAGES) $(`page-${p}`).hidden = p !== name;
  document.querySelectorAll(".nav-item").forEach((b) => {
    if (b.dataset.page === name) b.setAttribute("aria-current", "page"); else b.removeAttribute("aria-current");
  });
  $("main").scrollTop = 0;
  if (push) history.replaceState(null, "", `#${name}`);
  // replay the entrance animation of the page that just appeared
  document.querySelectorAll(`#page-${name} .reveal`).forEach((e) => { e.style.animation = "none"; void e.offsetWidth; e.style.animation = ""; });
  refreshPage();
  // Away card: one `tailscale status` per visit (and on Check again), never on the 4 s timer
  if (name === "devices" || name === "unlock") loadAway();
  if (name === "devices") loadCompanions();
}
document.querySelectorAll(".nav-item").forEach((b) => b.addEventListener("click", () => go(b.dataset.page)));
document.addEventListener("keydown", (e) => {
  // Ctrl+1..6 jumps between pages; Esc closes the window (the tray keeps Aura running)
  if (e.ctrlKey && /^[1-6]$/.test(e.key)) { e.preventDefault(); go(PAGES[+e.key - 1]); }
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s" && page === "lock") { e.preventDefault(); saveLock(); }
  if (e.key === "Escape" && !e.defaultPrevented && !lockDirty && !recording && !$("dlg").open && document.activeElement?.tagName !== "SELECT") {
    win.close();
  }
});

// ------------------------------------------------------------------ shared state
let A = null;          // aura_state
let LS = null;         // auto-lock status
let cfg = null;        // auto-lock config
let HS = null;         // handoff state
let paired = null, pairedError = null;

async function loadAura() {
  try { A = await invoke("aura_state"); renderAura(); }
  catch (e) { $("uHeroTitle").textContent = "Couldn't read Aura's state"; $("uHeroSub").textContent = String(e); $("uHero").dataset.tone = "warning"; }
}

// ------------------------------------------------------------------ UNLOCK page
function deviceChip(d) {
  if (!d.paired) return chip("Not paired", "neutral");
  if (d.presence === "Nearby") return chip("Nearby", "success");
  if (d.presence === "Away") return chip("Away", "warning");
  if (d.linkUp) return chip("Linked", "success");
  return chip("Paired", "primary");
}
function fillDevice(root, d, fallbackName) {
  root.querySelector('[data-k="name"]').textContent = d.name || fallbackName;
  root.querySelector('[data-k="desc"]').textContent = d.paired ? d.linkText : `Not paired yet. Pair the ${fallbackName.toLowerCase()} in the Aura app on it.`;
  const slot = root.querySelector('[data-k="chip"]');
  const c = deviceChip(d);
  if (slot.textContent !== c.textContent) slot.replaceChildren(c);
}
function fmtWhen(ms) {
  const d = new Date(ms), now = new Date();
  const t = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(now) - day(d)) / 86400000);
  if (diff === 0) return `Today ${t}`;
  if (diff === 1) return `Yesterday ${t}`;
  return `${d.toLocaleDateString([], { day: "numeric", month: "short" })} ${t}`;
}
function renderApprovals() {
  const box = $("approvals");
  const items = A.approvals || [];
  if (!items.length) {
    const e = el("div", "empty");
    const ic = el("div", "empty-icon"); ic.append(icon("clock")); e.append(ic);
    e.append(el("h3", null, "No sign-in requests yet"), el("p", null, "When you reach the Windows sign-in screen, the answer shows up here."));
    box.replaceChildren(e);
    return;
  }
  box.replaceChildren(...items.map((a) => {
    const row = el("div", "row approval");
    const denied = a.result.startsWith("Denied");
    const i = icon(a.ok ? "check" : denied ? "close" : "clock", "ic row-icon " + (a.ok ? "ok" : "no"));
    const t = el("span", "row-text");
    t.append(el("span", "row-label", a.result), el("span", "row-desc", fmtWhen(a.atMs) + (a.tookMs != null ? ` · ${a.tookMs} ms` : "")));
    row.append(i, t);
    return row;
  }));
}
function renderUnlockHero() {
  const hero = $("uHero");
  let tone = "neutral", ic = "unlock", title, sub;
  const near = [A.phone, A.watch].filter((d) => d.paired && (d.presence === "Nearby" || d.linkUp));
  if (A.serviceError) {
    tone = "warning"; ic = "warning"; title = "Aura's sign-in service didn't start"; sub = A.serviceError;
  } else if (!A.phone.paired && !A.watch.paired) {
    title = "No phone or watch paired yet"; sub = "Open Devices and choose Add a phone: Aura shows a QR code for the phone app to scan.";
  } else if (!A.handsFree) {
    ic = "lock"; title = "Hands-free sign-in is off"; sub = "The Windows sign-in screen asks for your PIN. Turn it on below.";
  } else if (near.length) {
    tone = "success"; ic = "shield"; title = "Ready to sign you in";
    sub = `${list(near.map((d) => d.name || (d.kind === "watch" ? "Your watch" : "Your phone")))} ${near.length > 1 ? "are" : "is"} nearby. Hands-free sign-in is on.`;
  } else {
    title = "Waiting for your phone or watch"; sub = "Hands-free sign-in is on. Aura checks again each time the sign-in screen appears.";
  }
  hero.dataset.tone = tone;
  $("uHeroIcon").setAttribute("href", `#i-${ic}`);
  if (hero.dataset.k !== title) {
    for (const id of ["uHeroTitle", "uHeroSub"]) { const e = $(id); e.classList.remove("swap"); void e.offsetWidth; e.classList.add("swap"); }
    hero.dataset.k = title;
  }
  $("uHeroTitle").textContent = title; $("uHeroSub").textContent = sub;
}
function renderAura() {
  if (!A) return;
  const live = $("sideMode");
  live.textContent = A.live ? "Live" : "Dry run";
  live.dataset.tone = A.live ? "warning" : "primary";
  live.title = A.live ? "Auto-lock is live: it really locks this PC" : "Auto-lock is in dry run: it never locks";
  renderUnlockHero();
  fillDevice($("uPhone"), A.phone, "Phone");
  fillDevice($("uWatch"), A.watch, "Watch");
  $("handsFree").checked = A.handsFree;
  renderApprovals();
  // devices page
  fillDevice($("dPhone"), A.phone, "Phone");
  fillDevice($("dWatch"), A.watch, "Watch");
  document.querySelector('[data-act="forget-phone"]').disabled = !A.phone.paired;
  document.querySelector('[data-act="forget-watch"]').disabled = !A.watch.paired;
  // about
  $("aboutVer").textContent = `Version ${A.version}`;
  $("autostart").checked = A.autostart;
  $("dataPath").textContent = A.dirs.data; $("dataPath").title = A.dirs.data;
  $("inboxPath").textContent = A.dirs.inbox; $("inboxPath").title = A.dirs.inbox;
  $("liveMode").checked = A.live;
  $("pauseBtn").textContent = A.paused ? "Resume now" : "Pause 1 hour";
}

$("handsFree").addEventListener("change", async (e) => {
  const want = e.target.checked;
  try { A.handsFree = await invoke("set_hands_free", { on: want }); renderAura(); }
  catch (err) { e.target.checked = !want; }
});

// ------------------------------------------------------------------ LOCK page (the auto-lock's settings)
let selected = new Set();
let lockDirty = false;
const key = (d) => d.name;
const fmtAddr = (a) => a.toString(16).padStart(12, "0").toUpperCase().match(/../g).join(":");
const KIND = {
  phone: ["Phone", "phone"], wearable: ["Watch", "watch"], audio: ["Audio", "headphones"],
  computer: ["Computer", "laptop"], peripheral: ["Accessory", "keyboard"], le: ["Bluetooth LE", "bluetooth"],
  other: ["Device", "bluetooth"], saved: ["Not paired right now", "bluetooth"],
};

function heroFor(s) {
  const live = s.live_mode;
  const modeLine = live ? "Live: this PC locks when your devices leave."
                        : "Dry run: you get a notification instead of a lock.";
  const v = s.verdict || {};
  const awayBit = (ms) => (ms > 0 ? ` You've been away ${secs(ms)} s.` : "");
  const nearby = (s.devices || []).filter((d) => d.presence === "present").map((d) => d.name);
  switch (v.kind) {
    case "no_devices":
      return paired && !paired.length
        ? { tone: "neutral", icon: "bluetooth", title: "Nothing to watch yet", sub: "Pair your phone or watch with this PC first." }
        : { tone: "neutral", icon: "bluetooth", title: "Pick a device to watch", sub: "Tick your phone or watch below, then Save." };
    case "present":
      return { tone: "success", icon: "shield", title: nearby.length ? `${list(nearby)} ${nearby.length > 1 ? "are" : "is"} nearby` : "Your devices are nearby", sub: modeLine };
    case "not_armed":
      return { tone: "neutral", icon: "timer", title: "Waiting to see your devices", sub: "Auto-lock starts watching once it has seen them nearby." };
    case "counting": {
      const left = Math.max(0, secs(v.need_ms - v.absent_ms));
      return { tone: "warning", icon: "timer", title: `Away for ${secs(v.absent_ms)} s`,
        sub: live ? `Locks in ${left} s unless a device comes back.` : `Would lock in ${left} s (dry run).`,
        meter: v.need_ms ? v.absent_ms / v.need_ms : 0 };
    }
    case "lock_explicit":
      return { tone: "warning", icon: "lock", title: live ? "Locking now" : "Would lock now", sub: live ? "Lock PC was pressed on your watch or phone." : "Dry run: nothing was locked." };
    case "lock":
      return { tone: "warning", icon: "lock", title: live ? "Locking now" : "Would lock now", sub: live ? "Your devices are away." : "Dry run: nothing was locked." };
    case "blocked": {
      const r = {
        session_locked: { tone: "neutral", icon: "lock", title: "This PC is locked", sub: "Nothing to do until you sign in." },
        bluetooth_off: { tone: "warning", icon: "bt-off", title: "Bluetooth is off", sub: "Auto-lock can't see your devices. Turn Bluetooth on in Windows quick settings." },
        paused: { tone: "neutral", icon: "pause", title: "Paused",
          sub: s.paused_left_secs != null ? `Back on in ${Math.max(1, Math.ceil(s.paused_left_secs / 60))} min. Resume early below.` : "Resume below." },
        unlock_grace: { tone: "neutral", icon: "shield", title: "You just signed in", sub: "Never locks in the first 60 seconds." },
        fullscreen: { tone: "neutral", icon: "play", title: "Holding for fullscreen", sub: "Won't lock while video or slides fill the screen." },
        cooldown: { tone: "neutral", icon: "clock", title: "Locked a moment ago", sub: "Waits 2 minutes between locks." },
        recent_input: { tone: "neutral", icon: "shield", title: "You are using this PC", sub: "Never locks within a minute of typing or moving the mouse." },
      }[v.reason] || { tone: "neutral", icon: "info", title: s.summary || "Holding", sub: "" };
      return { ...r, sub: r.sub + awayBit(v.absent_ms) };
    }
    default:
      return { tone: "neutral", icon: "info", title: s.summary || "Starting…", sub: modeLine };
  }
}
let lastHeroKey = "";
function renderHero(s) {
  const h = heroFor(s);
  const hero = $("hero");
  hero.dataset.tone = h.tone;
  $("heroIcon").setAttribute("href", `#i-${h.icon}`);
  const k = h.tone + h.icon + h.title.replace(/\d+/g, "");
  if (k !== lastHeroKey) {
    for (const id of ["heroTitle", "heroSub"]) { const e = $(id); e.classList.remove("swap"); void e.offsetWidth; e.classList.add("swap"); }
    lastHeroKey = k;
  }
  $("heroTitle").textContent = h.title;
  $("heroSub").textContent = h.sub;
  const m = $("meter");
  m.hidden = h.meter == null;
  if (h.meter != null) m.firstElementChild.style.setProperty("--p", Math.min(1, h.meter).toFixed(3));
  $("liveMode").checked = s.live_mode;
  $("liveD").textContent = s.live_mode ? "On: this PC really locks when your devices leave" : "Off = dry run: you get a notification instead of a lock";
  const paused = s.paused_left_secs != null;
  $("pauseBtn").textContent = paused ? "Resume now" : "Pause 1 hour";
  $("pauseD").textContent = paused ? `Back on in ${Math.max(1, Math.ceil(s.paused_left_secs / 60))} min` : "For one hour, then it resumes by itself";
  const side = $("sideMode");
  side.textContent = s.live_mode ? "Live" : "Dry run";
  side.dataset.tone = s.live_mode ? "warning" : "primary";
}

function emptyState({ tone, iconName, title, text, action, onAction }) {
  const box = el("div", "empty");
  if (tone) box.dataset.tone = tone;
  const ic = el("div", "empty-icon"); ic.append(icon(iconName)); box.append(ic);
  box.append(el("h3", null, title), el("p", null, text));
  if (action) {
    const b = el("button", "btn"); b.type = "button"; b.append(icon("refresh", "ic sm"), document.createTextNode(" " + action));
    b.onclick = onAction; box.append(b);
  }
  return box;
}
let rows = new Map();
function renderDevices() {
  const box = $("devices");
  if (paired === null) return;
  rows = new Map();
  if (pairedError && !paired.length) {
    box.replaceChildren(emptyState({ tone: "warning", iconName: "warning", title: "Couldn't read your Bluetooth devices", text: String(pairedError), action: "Try again", onAction: loadPaired }));
    return;
  }
  if (!paired.length) {
    box.replaceChildren(emptyState({ iconName: "bluetooth", title: "No paired devices yet",
      text: "Pair your phone or watch in Windows Settings › Bluetooth & devices, then check again.", action: "Check again", onAction: loadPaired }));
    return;
  }
  box.replaceChildren(...paired.map((d, i) => {
    const row = el("label", "row clickable device");
    const id = `dev-${i}`;
    row.htmlFor = id;
    const cb = el("input", "check");
    cb.type = "checkbox"; cb.id = id; cb.checked = selected.has(key(d));
    cb.onchange = () => { cb.checked ? selected.add(key(d)) : selected.delete(key(d)); markDirty(); refreshChips(); };
    const [kindName, kindIcon] = KIND[d.kind] || KIND.other;
    const di = el("span", "dev-icon"); di.append(icon(kindIcon));
    const txt = el("span", "row-text");
    const addrs = [d.classic != null ? fmtAddr(d.classic) : null, ...(d.le || []).map((a) => "LE " + fmtAddr(a))].filter(Boolean).join("  ");
    const desc = el("span", "row-desc", kindName);
    txt.append(el("span", "row-label", d.name), desc);
    if (addrs) row.title = `${d.name}\n${addrs}`;
    const slot = el("span", "chip-slot");
    row.append(cb, di, txt, slot);
    rows.set(key(d), { row, cb, slot, d, desc, kindName, last: "" });
    return row;
  }));
  refreshChips();
}
function refreshChips() {
  const live = new Map((LS?.devices || []).map((d) => [d.name, d]));
  for (const r of rows.values()) {
    r.cb.checked = selected.has(key(r.d));
    const l = live.get(r.d.name);
    const watched = selected.has(key(r.d)) && l;
    const signal = !watched ? "" : l.rssi != null ? ` · signal ${String(l.rssi).replace("-", "−")} dBm` : l.connected ? " · connected" : "";
    r.desc.textContent = r.kindName + signal;
    let c = null;
    if (selected.has(key(r.d))) {
      if (!l) c = chip("Save to watch");
      else if (l.presence === "present") c = chip("Nearby", "success");
      else if (l.presence === "absent") c = chip("Away", "warning");
      else c = chip("Checking", "neutral");
    }
    const sig = c ? c.textContent : "";
    if (sig === r.last) continue;
    r.last = sig;
    r.slot.replaceChildren(...(c ? [c] : []));
  }
}
async function loadPaired() {
  pairedError = null;
  try { paired = await invoke("list_paired"); }
  catch (e) { paired = []; pairedError = e; }
  for (const d of cfg?.devices || []) if (!paired.some((p) => key(p) === key(d))) paired.push({ ...d, kind: "saved" });
  renderDevices();
  if (LS) renderHero(LS);
}

function setRangeFill() {
  const r = $("rssi");
  const pct = ((r.value - r.min) / (r.max - r.min)) * 100;
  r.style.setProperty("--fill", pct + "%");
  $("rssival").textContent = `${String(r.value).replace("-", "−")} dBm`;
  $("rssirow").classList.toggle("is-disabled", !$("userssi").checked);
  $("rssi").disabled = !$("userssi").checked;
  $("offwristRow").classList.toggle("is-disabled", !$("offwrist").checked);
  $("offwristSecs").disabled = !$("offwrist").checked;
}
function fillForm(c) {
  $("absent").value = c.absent_secs;
  const r = document.querySelector(`input[name=when][value=${c.lock_when}]`);
  if (r) r.checked = true;
  $("userssi").checked = c.use_rssi;
  $("rssi").value = c.rssi_threshold;
  $("fullscreen").checked = c.block_on_fullscreen;
  $("ping").value = c.ping_interval_secs;
  $("idleGuard").value = c.idle_guard_secs;
  $("minMisses").value = c.min_misses;
  $("singleSignal").value = c.single_signal_secs;
  $("offwrist").checked = c.offwrist_hint;
  $("offwristSecs").value = c.offwrist_lock_secs;
  $("linkSignals").checked = c.link_signals;
  $("liveMode").checked = c.live_mode;
  selected = new Set(c.devices.map(key));
  setRangeFill();
}
function markDirty() {
  lockDirty = true;
  $("save").disabled = false; $("revert").hidden = false;
  note("saved", "Unsaved changes", "", true);
}
function markClean() { lockDirty = false; $("save").disabled = true; $("revert").hidden = true; }
function clampInput(input) {
  const v = Math.round(+input.value);
  const lo = +input.min, hi = +input.max;
  const c = Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : lo;
  if (String(c) !== input.value) input.value = c;
}
const NUMS = ["absent", "ping", "idleGuard", "minMisses", "singleSignal", "offwristSecs"];
for (const id of NUMS) {
  $(id).addEventListener("input", markDirty);
  $(id).addEventListener("blur", () => clampInput($(id)));
}
for (const id of ["userssi", "rssi", "fullscreen", "offwrist", "linkSignals"]) $(id).addEventListener("input", () => { setRangeFill(); markDirty(); });
document.querySelectorAll("input[name=when]").forEach((r) => r.addEventListener("change", markDirty));

async function saveLock() {
  if (!lockDirty) return;
  NUMS.forEach((id) => clampInput($(id)));
  const devices = (paired || []).filter((d) => selected.has(key(d))).map((d) => ({ name: d.name, classic: d.classic, le: d.le }));
  const next = {
    ...cfg, devices,
    absent_secs: +$("absent").value,
    lock_when: document.querySelector("input[name=when]:checked").value,
    use_rssi: $("userssi").checked,
    rssi_threshold: +$("rssi").value,
    block_on_fullscreen: $("fullscreen").checked,
    ping_interval_secs: +$("ping").value,
    idle_guard_secs: +$("idleGuard").value,
    min_misses: +$("minMisses").value,
    single_signal_secs: +$("singleSignal").value,
    offwrist_hint: $("offwrist").checked,
    offwrist_lock_secs: +$("offwristSecs").value,
    link_signals: $("linkSignals").checked,
  };
  $("save").disabled = true;
  try {
    await invoke("save_config", { config: next });
    cfg = await invoke("get_config");
    fillForm(cfg);
    markClean();
    note("saved", "Saved", "success");
    refreshChips();
  } catch (e) {
    $("save").disabled = false;
    note("saved", `Couldn't save: ${e}`, "warning", true);
  }
}
$("save").onclick = saveLock;
$("revert").onclick = () => { fillForm(cfg); markClean(); note("saved", ""); refreshChips(); };

$("liveMode").addEventListener("change", async (e) => {
  const want = e.target.checked;
  try {
    const now = await invoke("set_live", { on: want });
    e.target.checked = now;
    if (cfg) cfg.live_mode = now;
    note("saved", now ? "Live: this PC will really lock" : "Dry run: nothing will be locked", now ? "warning" : "success");
    loadAura();
  } catch (err) { e.target.checked = !want; note("saved", `Couldn't switch: ${err}`, "warning", true); }
});
$("pauseBtn").addEventListener("click", async () => { await invoke("pause_toggle"); setTimeout(loadLock, 1200); });

async function loadLock() {
  try {
    cfg = await invoke("get_config");
    if (!lockDirty) fillForm(cfg);
    LS = await invoke("get_status");
    renderHero(LS); refreshChips();
  } catch (e) {
    $("heroTitle").textContent = "Couldn't reach the auto-lock";
    $("heroSub").textContent = String(e);
    $("hero").dataset.tone = "warning";
  }
}

// ------------------------------------------------------------------ HANDOFF page
let current = null;   // Handoff settings (camelCase)
let recording = null; // the recorder button that is listening

function showKeys(btn, accel) {
  btn.replaceChildren();
  if (!accel) { const s = el("span", "hint", "Off"); btn.append(s); return; }
  accel.split("+").forEach((k, i) => {
    if (i) btn.append(el("span", "plus", "+"));
    btn.append(el("kbd", null, k === "Super" ? "Win" : k));
  });
}
const KEY_NAMES = { Space: "Space", Enter: "Enter", Backquote: "`", Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/" };
function keyName(code) {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  return KEY_NAMES[code] ?? null;
}
function makeRecorder(btn, field, errId, label, canClear) {
  const show = () => showKeys(btn, current ? current[field] : "");
  const stop = () => { recording = null; btn.classList.remove("recording", "nudge"); show(); };
  const hint = (t, nudge) => { btn.replaceChildren(el("span", "hint", t)); btn.classList.toggle("nudge", !!nudge); };
  btn.addEventListener("click", () => {
    if (recording === btn) { stop(); return; }
    if (recording) recording.blur();
    recording = btn; btn.classList.add("recording");
    hint(canClear ? "Press keys, Backspace turns it off" : "Press the new keys…");
    btn.setAttribute("aria-label", `Recording ${label}. Press the new shortcut, or Esc to cancel.`);
  });
  btn.addEventListener("blur", () => { if (recording === btn) stop(); });
  btn.addEventListener("keydown", (e) => {
    if (recording !== btn) return;
    e.preventDefault(); e.stopPropagation();
    if (e.key === "Escape") { stop(); return; }
    if (canClear && e.key === "Backspace") { recording = null; btn.classList.remove("recording", "nudge"); saveHandoff({ [field]: "" }, errId); return; }
    const k = keyName(e.code);
    if (!k) return;
    const mods = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    if (!mods.some((m) => m !== "Shift")) { hint("Add Ctrl, Alt or Win", true); return; }
    recording = null; btn.classList.remove("recording", "nudge");
    saveHandoff({ [field]: [...mods, k].join("+") }, errId);
  });
  return show;
}
const showHotkey = makeRecorder($("hotkey"), "hotkey", "hotkeyError", "the send shortcut", false);
const showSnip = makeRecorder($("snipKey"), "snipHotkey", "snipError", "the snip shortcut", true);

function renderHandoffHero(state) {
  const hero = $("hHero");
  const q = state.queueLen || 0;
  let tone, ic, title, sub;
  if (state.peer) {
    tone = q ? "warning" : "success"; ic = q ? "clock" : "phone";
    title = `${state.peer.name} is ready`;
    sub = q ? `${q} waiting to send. Going now.` : state.incomingConnected
      ? `Connected through ${state.backend}. Sending and receiving.`
      : `Connected through ${state.backend}. Receiving is still connecting.`;
  } else if (state.devicesError) {
    tone = "warning"; ic = "warning"; title = `${state.backend} isn't answering`;
    sub = "Aura's service is still starting. Try again in a moment.";
  } else if (!state.devices?.length) {
    tone = "neutral"; ic = "phone"; title = "No phone paired yet";
    sub = "Pair your phone in the Aura phone app first.";
  } else {
    tone = q ? "warning" : "neutral"; ic = q ? "clock" : "phone"; title = "Phone not reachable";
    sub = q ? `${q} waiting. They send by themselves when the phone is back.` : "Anything you send now waits here until the phone is back.";
  }
  hero.dataset.tone = tone;
  $("hHeroIcon").setAttribute("href", `#i-${ic}`);
  const k = tone + ic + title;
  if (hero.dataset.k !== k) {
    for (const id of ["hPeer", "hSub"]) { const e = $(id); e.classList.remove("swap"); void e.offsetWidth; e.classList.add("swap"); }
    hero.dataset.k = k;
  }
  $("hPeer").textContent = title; $("hSub").textContent = sub;
}
function renderHandoff(state) {
  HS = state; current = state.settings;
  renderHandoffHero(state);
  if (!recording) { showHotkey(); showSnip(); }
  $("openInBrave").checked = current.openInBrave;
  $("youtubeTimestamp").checked = current.youtubeTimestamp;
  $("suggest").checked = current.suggest;
  $("phoneBubble").checked = current.phoneBubble;
  $("phoneLinksInApp").checked = current.phoneLinksInApp;
  $("clipboardSync").checked = current.clipboardSync;
}
async function loadHandoff() {
  try { renderHandoff(await invoke("get_state")); }
  catch (e) {
    $("hHero").dataset.tone = "warning";
    $("hHeroIcon").setAttribute("href", "#i-warning");
    $("hPeer").textContent = "Couldn't read Handoff's state"; $("hSub").textContent = String(e);
  }
}
async function saveHandoff(patch, errId) {
  const next = { ...current, ...patch };
  const errEl = errId ? $(errId) : null;
  try {
    current = await invoke("save_settings", { new: next });
    if (errEl) errEl.hidden = true;
  } catch (e) {
    if (errEl) { errEl.textContent = String(e); errEl.hidden = false; }
  }
  if (!recording) { showHotkey(); showSnip(); }
}
for (const id of ["openInBrave", "youtubeTimestamp", "suggest", "clipboardSync", "phoneBubble", "phoneLinksInApp"]) {
  $(id).addEventListener("change", (e) => saveHandoff({ [id]: e.target.checked }));
}
$("openInbox").addEventListener("click", () => invoke("open_folder", { which: "inbox" }));
// Files dropped on the window go to the phone (a tray icon cannot take drops on Windows).
window.__TAURI__.webview.getCurrentWebview().onDragDropEvent(async (e) => {
  const d = $("drop");
  if (page !== "handoff") return;
  if (e.payload.type === "over" || e.payload.type === "enter") d.classList.add("hot"); else d.classList.remove("hot");
  if (e.payload.type === "drop" && e.payload.paths?.length) {
    try {
      const n = await invoke("send_paths", { paths: e.payload.paths });
      $("dropD").textContent = n ? `Sending ${n} file${n > 1 ? "s" : ""}…` : "Only files can be sent (not folders)";
    } catch (err) { $("dropD").textContent = String(err); }
  }
});

// ------------------------------------------------------------------ dialog helper
// waitFor: optional async check polled every 1.5 s while the dialog is open; when it returns true the dialog says
// "Paired" and closes by itself (the QR must leave the screen as soon as the phone has used it)
function ask({ title, text, code, qr, expiresAt, ok = "OK", cancel = "Cancel", danger, waitFor }) {
  return new Promise((resolve) => {
    const dlg = $("dlg");
    $("dlgTitle").textContent = title;
    const t = $("dlgText"); t.replaceChildren(...[].concat(text).map((p) => el("p", null, p)));
    const c = $("dlgCode"); c.hidden = !code;
    // one span per 4-letter group so a long code wraps between groups, never inside one
    c.replaceChildren(...(code ? code.split("-").map((g) => el("span", "g", g)) : []));
    const q = $("dlgQr"); q.hidden = !qr;
    let timer = null;
    if (qr) {
      // the SVG comes from our own Rust command (qrcode crate); parse it, never innerHTML it
      const doc = new DOMParser().parseFromString(qr, "image/svg+xml").documentElement;
      const cap = el("div", "qr-cap", "");
      q.replaceChildren(document.importNode(doc, true), cap);
      const tick = () => {
        const left = Math.max(0, Math.round(expiresAt - Date.now() / 1000));
        cap.textContent = left ? `Scan with the Aura phone app. Valid for ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}.` : "This QR expired. Close and make a new one.";
        q.classList.toggle("expired", !left);
      };
      tick(); timer = setInterval(tick, 1000);
    }
    $("dlgOk").textContent = ok; $("dlgOk").hidden = !ok;
    $("dlgCancel").textContent = cancel;
    let poll = null;
    if (waitFor) {
      poll = setInterval(async () => {
        let hit = false;
        try { hit = await waitFor(); } catch (_) {}
        if (!hit) return;
        clearInterval(poll); poll = null;
        if (timer) { clearInterval(timer); timer = null; }
        q.hidden = true; c.hidden = true;
        $("dlgTitle").textContent = "Paired";
        t.replaceChildren(el("p", null, "The phone is connected. You can put it down."));
        setTimeout(() => done(true), 1500);
      }, 1500);
    }
    const done = (v) => { if (timer) clearInterval(timer); if (poll) clearInterval(poll); dlg.close(); $("dlgOk").onclick = null; $("dlgCancel").onclick = null; dlg.onclose = null; resolve(v); };
    $("dlgOk").onclick = () => done(true);
    $("dlgCancel").onclick = () => done(false);
    dlg.onclose = () => resolve(false);
    dlg.showModal();
    $("dlgCancel").focus();
  });
}
const needRestart = () => { $("restartBanner").hidden = false; };
$("restartNow").addEventListener("click", () => invoke("restart_app"));
$("restartBtn").addEventListener("click", async () => {
  if (await ask({ title: "Restart Aura?", text: "Aura quits and starts again in a few seconds. Auto-lock, hotkeys and sign-in answers pause meanwhile.", ok: "Restart" })) invoke("restart_app");
});
$("exitBtn").addEventListener("click", async () => {
  if (await ask({ title: "Exit Aura?", text: "Auto-lock, Handoff and hands-free sign-in stop until you start Aura again from the Start menu.", ok: "Exit Aura" })) invoke("quit_app");
});

// ------------------------------------------------------------------ DEVICES page actions
document.querySelector('[data-act="forget-phone"]').addEventListener("click", async () => {
  if (!(await ask({ title: "Forget the phone?", text: ["Hands-free sign-in and Lock PC from the phone stop working until you pair it again.", "Nothing is deleted: the pairing files move to a backup folder."], ok: "Forget phone" }))) return;
  try { const r = await invoke("forget_device", { kind: "phone" }); needRestart(); loadAura();
    await ask({ title: "Phone forgotten", text: r.backup ? `Backup: ${r.backup}` : "There was nothing to forget.", ok: "", cancel: "Close" }); }
  catch (e) { await ask({ title: "Couldn't forget the phone", text: String(e), ok: "", cancel: "Close" }); }
});
document.querySelector('[data-act="forget-watch"]').addEventListener("click", async () => {
  if (!(await ask({ title: "Forget the watch?", text: ["Lock PC and Unlock PC from the watch stop working until you pair it again.", "Nothing is deleted: the pairing files move to a backup folder."], ok: "Forget watch" }))) return;
  try { const r = await invoke("forget_device", { kind: "watch" }); needRestart(); loadAura();
    await ask({ title: "Watch forgotten", text: r.backup ? `Backup: ${r.backup}` : "There was nothing to forget.", ok: "", cancel: "Close" }); }
  catch (e) { await ask({ title: "Couldn't forget the watch", text: String(e), ok: "", cancel: "Close" }); }
});
document.querySelector('[data-act="repair-phone"]').addEventListener("click", async () => {
  if (!(await ask({ title: "Replace your MAIN phone?", text: ["This is only for a new main phone or a broken pairing. Your current main phone stops working with this PC (no hands-free sign-in, no Lock PC) until it scans the new QR. Your PIN keeps working.", "To add a SECOND phone instead, cancel and use \"Add another phone\" below."], ok: "Replace main phone" }))) return;
  try {
    const r = await invoke("repair_phone");
    needRestart(); loadAura();
    // the running link still holds the old pairing: once the phone has scanned, Aura restarts to use the new one
    if (await ask({ title: "Pair your main phone", text: ["Open the Aura phone app and tap Scan QR (or type the code). When the phone says it is paired, press \"Scanned, restart\".", r.backup ? `Old pairing backed up in ${r.backup}` : ""].filter(Boolean), code: r.code, qr: r.qrSvg, expiresAt: r.expiresAt, ok: "Scanned, restart", cancel: "Later" })) invoke("restart_app");
  } catch (e) { await ask({ title: "Couldn't re-pair", text: String(e), ok: "", cancel: "Close" }); }
});
document.querySelector('[data-act="pair-watch"]').addEventListener("click", async () => {
  if (!(await ask({ title: "Pair a watch", text: ["Open the Aura watch app and tap Pair with PC. It listens for 60 seconds.", "Then press Start here."], ok: "Start" }))) return;
  try {
    const r = await invoke("watch_pair_start");
    const same = await ask({ title: "Same code on the watch?", text: "Check that the watch shows exactly this code before you pin it.", code: r.fingerprint, ok: "Yes, pin it", cancel: "No" });
    await invoke("watch_pair_confirm", { accept: same });
    if (same) { needRestart(); loadAura(); }
  } catch (e) { await ask({ title: "Watch not paired", text: String(e), ok: "", cancel: "Close" }); }
});

// ------------------------------------------------------------------ companion phones (many devices, phase 1)
let compTimer = null;
async function loadCompanions() {
  const box = $("dCompanions");
  try {
    const st = await invoke("companions_state");
    const rows = [];
    const row = (name, desc, role, up, onRemove) => {
      const r = el("div", "row");
      const t = el("span", "row-text");
      t.append(el("span", "row-label", name), el("span", "row-desc", desc));
      r.append(t, chip(role, role === "Main" ? "success" : "neutral"), chip(up ? "Connected" : "Not connected", up ? "success" : "neutral"));
      if (onRemove) { const b = el("button", "btn", "Remove"); b.type = "button"; b.dataset.act = "remove-companion"; b.onclick = onRemove; r.append(b); }
      return r;
    };
    if (st.primary.paired) rows.push(row(st.primary.name || "Main phone", "Unlocks this PC and does everything a companion does. Forget it above.", "Main", st.primary.connected, null));
    for (const c of st.companions) {
      rows.push(row(c.name, c.model ? `${c.model}${c.via ? " via " + c.via : ""}` : "Companion: clipboard, files, Continue, remote", "Companion", c.connected, async () => {
        if (!(await ask({ title: `Remove ${c.name}?`, text: ["This phone stops sharing with this PC at once. The main phone and the unlock are not touched.", "Nothing is deleted: its pairing moves to a backup folder."], ok: "Remove phone" }))) return;
        try { await invoke("companion_remove", { id: c.id }); } catch (e) { await ask({ title: "Couldn't remove the phone", text: String(e), ok: "", cancel: "Close" }); }
        loadCompanions();
      }));
    }
    if (!rows.length) rows.push(el("div", "row", "No phones paired yet."));
    box.replaceChildren(...rows);
  } catch (e) {
    box.replaceChildren(el("div", "row", `Couldn't read the phones: ${e}`));
  }
  clearTimeout(compTimer);
  if (page === "devices") compTimer = setTimeout(loadCompanions, 5000);
}
document.querySelector('[data-act="add-companion"]').addEventListener("click", async () => {
  if (!(await ask({ title: "Add another phone?", text: ["Aura makes a separate pairing code and QR for the second phone. It can share the clipboard and files, use Continue and control this PC, but it can never unlock it.", "Your main phone and the unlock are not touched. Have the second phone in your hand with the Aura app open."], ok: "Show QR code" }))) return;
  try {
    const n = ((await invoke("companions_state")).companions.length) + 2;
    const r = await invoke("companion_add", { name: `Phone ${n}` });
    loadCompanions();
    await ask({ title: "Add the second phone", text: "In the Aura app on the second phone open Settings, Laptops, Add a laptop and scan this QR (or type the code). No restart is needed.", code: r.code, qr: r.qrSvg, expiresAt: r.expiresAt, ok: "", cancel: "Done",
      waitFor: async () => ((await invoke("companions_state")).companions.find((x) => x.id === r.id) || {}).connected });
    loadCompanions();
  } catch (e) { await ask({ title: "Couldn't add the phone", text: String(e), ok: "", cancel: "Close" }); }
});

// ------------------------------------------------------------------ AWAY (item 17): Devices card + Unlock section
let AW = null, awayBusy = false;
const AWAY_CHIP = { ready: ["Ready", "success"], missing: ["Not installed", "warning"], off: ["Off", "warning"], login: ["Sign in", "warning"],
  "no-phone": ["Phone not found", "warning"], "phone-offline": ["Phone offline", "neutral"], starting: ["Starting", "neutral"], unknown: ["No answer", "warning"], disabled: ["Off", "neutral"] };
const REMOTE_ICON = { lock: "lock", unlock: "unlock", "unlock-refused": "close", "unlock-expired": "clock", fetch: "handoff", relock: "timer" };
async function loadAway() {
  if (awayBusy) return;
  awayBusy = true; $("awayRefresh").disabled = true;
  try { AW = await invoke("away_state"); renderAway(); }
  catch (e) { $("awayTitle").textContent = "Couldn't check Tailscale"; $("awayText").textContent = String(e); }
  finally { awayBusy = false; $("awayRefresh").disabled = false; }
}
function renderAway() {
  if (!AW) return;
  const c = AW.card;
  $("dAway").dataset.code = c.code;
  $("awayTitle").textContent = c.title; $("awayText").textContent = c.text;
  const [ct, tone] = AWAY_CHIP[c.code] || ["", "neutral"];
  const slot = $("awayChip"); const ch = chip(ct, tone);
  if (slot.textContent !== ct) slot.replaceChildren(ch);
  const act = $("awayAction");
  act.hidden = c.action === "none";
  act.textContent = c.action === "install" ? "Get Tailscale" : c.code === "login" ? "Sign in to Tailscale" : c.code === "off" ? "Turn on Tailscale" : "Open Tailscale";
  act.dataset.which = c.action;
  $("awayWinget").hidden = c.code !== "missing";
  $("awayPc").textContent = AW.thisPc ? (AW.listening ? `${AW.thisPc} (listening)` : AW.thisPc) : "-";
  $("awayPhone").textContent = AW.phoneIp ? `${AW.phoneIp}${AW.phoneOnline === true ? " (online)" : AW.phoneOnline === false ? " (offline)" : ""}` : "learned when the phone connects";
  $("awayMode").checked = AW.awayMode;
  $("uAwayMeta").textContent = AW.awayMode ? (c.code === "ready" ? "Ready over Tailscale" : c.title) : "Away mode is off";
  $("remoteUnlock").checked = AW.remoteUnlock;
  $("remotePower").checked = AW.remotePower !== false;
  $("stayReachable").checked = AW.stayReachable !== false;
  $("shareRecent").checked = AW.shareRecent !== false;
  $("relock").checked = AW.relockAfterRemote;
  if (document.activeElement !== $("relockMin")) $("relockMin").value = AW.relockMinutes;
  $("relockRow").classList.toggle("is-disabled", !AW.relockAfterRemote);
  $("relockMin").disabled = !AW.relockAfterRemote;
  const box = $("remoteLog");
  const items = AW.recent || [];
  box.hidden = !items.length;
  box.replaceChildren(...items.map((r) => {
    const row = el("div", "row approval");
    const ok = r.kind === "unlock" || r.kind === "lock" || r.kind === "fetch" || r.kind === "relock";
    const t = el("span", "row-text");
    t.append(el("span", "row-label", r.detail), el("span", "row-desc", fmtWhen(r.atMs)));
    row.append(icon(REMOTE_ICON[r.kind] || "info", "ic row-icon " + (ok ? "ok" : "no")), t);
    return row;
  }));
}
async function saveAway(patch, input) {
  try {
    const changedMode = await invoke("set_away", { patch });
    if (changedMode) needRestart();
    await loadAway();
  } catch (e) { if (input && input.type === "checkbox") input.checked = !input.checked; }
}
$("awayRefresh").addEventListener("click", loadAway);
$("awayAction").addEventListener("click", async () => {
  try { $("awayText").textContent = await invoke("tailscale_action", { which: $("awayAction").dataset.which }); } catch (e) { $("awayText").textContent = String(e); }
});
$("awayWinget").addEventListener("click", async () => {
  try { await navigator.clipboard.writeText(AW.winget); $("awayText").textContent = `Copied: ${AW.winget} (paste it into Terminal)`; }
  catch { $("awayText").textContent = AW.winget; }
});
$("awayMode").addEventListener("change", (e) => saveAway({ awayMode: e.target.checked }, e.target));
$("remoteUnlock").addEventListener("change", (e) => saveAway({ remoteUnlock: e.target.checked }, e.target));
$("remotePower").addEventListener("change", (e) => saveAway({ remotePower: e.target.checked }, e.target));
$("stayReachable").addEventListener("change", (e) => saveAway({ stayReachable: e.target.checked }, e.target));
$("shareRecent").addEventListener("change", (e) => saveAway({ shareRecent: e.target.checked }, e.target));
$("relock").addEventListener("change", (e) => saveAway({ relockAfterRemote: e.target.checked }, e.target));
$("relockMin").addEventListener("change", (e) => {
  const v = Math.min(240, Math.max(1, Math.round(+e.target.value || 10)));
  e.target.value = v;
  saveAway({ relockMinutes: v }, e.target);
});

// ------------------------------------------------------------------ ABOUT page
$("autostart").addEventListener("change", async (e) => {
  const want = e.target.checked;
  try { e.target.checked = await invoke("set_autostart", { on: want }); }
  catch (err) { e.target.checked = !want; }
});
$("openData").addEventListener("click", () => invoke("open_folder", { which: "data" }));

// ------------------------------------------------------------------ REMOTE page (item 15)
let R = null;
async function budsRefresh() {
  try {
    const s = await invoke("buds_state");
    $("budsFollow").checked = s.follow;
    $("budsNote").textContent = s.found ? (s.onPc ? "On this PC. " : "Not on this PC. ") + s.note : s.note;
    $("budsMove").disabled = !s.found || s.onPc;
  } catch {}
}
$("budsFollow").addEventListener("change", async (e) => {
  try { await invoke("buds_follow", { on: e.target.checked }); } catch { e.target.checked = !e.target.checked; }
  budsRefresh();
});
async function headsRefresh() {
  try {
    const s = await invoke("heads_state");
    $("headsOn").checked = s.on;
    $("headsSnooze").value = s.snooze ? "snooze" : "manual";
    $("headsSnoozeRow").hidden = !s.on;
    $("headsShowRow").hidden = !(s.on && s.hidden);
    $("headsShowDesc").textContent = s.snooze ? "You threw it on the X; it comes back by itself in 30 minutes" : "You threw it on the X; it stays hidden until you turn it back on";
  } catch {}
}
headsRefresh();
setInterval(headsRefresh, 5000);
$("headsOn").addEventListener("change", async (e) => {
  try { await invoke("heads_set", { on: e.target.checked }); } catch { e.target.checked = !e.target.checked; }
  headsRefresh();
});
$("headsSnooze").addEventListener("change", async (e) => {
  try { await invoke("heads_snooze", { snooze: e.target.value === "snooze" }); } catch {}
  headsRefresh();
});
$("headsShow").addEventListener("click", async () => {
  try { await invoke("heads_show"); } catch {}
  headsRefresh();
});
$("budsMove").addEventListener("click", async () => {
  $("budsMove").disabled = true; $("budsNote").textContent = "Connecting...";
  try { await invoke("buds_move_here"); } catch {}
  setTimeout(budsRefresh, 4000);
});
setInterval(budsRefresh, 4000); budsRefresh();
async function loadRemote() {
  // each section refreshes on its own: one slow command must never leave the others blank
  mirrorRefresh();
  sunRefresh();
  try { R = await invoke("remote_state"); renderRemote(); } catch {}
}
function renderRemote() {
  if (!R) return;
  $("rRing").checked = R.ring; $("rFull").checked = R.fullVolume; $("rMedia").checked = R.media; $("rPres").checked = R.presenter; $("rInput").checked = R.input;
  $("ringPhone").textContent = R.phoneRinging ? "Stop ringing" : "Ring my phone";
  $("rPhoneDesc").textContent = R.phoneNote || (R.phonePaired ? "Rings at full volume, even on silent, until you touch it" : "Pair your phone first (Devices)");
  $("ringPhone").disabled = !R.phonePaired;
  $("rTitle").textContent = R.now.title || "Nothing playing";
  $("rArtist").textContent = R.now.title ? (R.now.artist || "Unknown artist") : "Start something on this PC";
  $("rChip").replaceChildren(chip(R.now.title ? (R.now.playing ? "Playing" : "Paused") : "Idle", R.now.playing ? "success" : null));
}
for (const [id, field] of [["rRing", "ring"], ["rFull", "fullVolume"], ["rMedia", "media"], ["rPres", "presenter"], ["rInput", "input"]]) {
  $(id).addEventListener("change", async (e) => {
    try { await invoke("set_remote", { patch: { [field]: e.target.checked } }); } catch { e.target.checked = !e.target.checked; }
    loadRemote();
  });
}
$("ringPhone").addEventListener("click", async () => {
  const b = $("ringPhone"); b.disabled = true;
  const want = !(R && R.phoneRinging);
  try { R.phoneNote = await invoke("ring_phone", { on: want }); } catch (err) { R.phoneNote = String(err); }
  b.disabled = false; await loadRemote();
});
$("watShow").addEventListener("click", async () => {
  const b = $("watShow"); b.disabled = true; const d = $("watDesc");
  try {
    if (await invoke("watch_running")) { await invoke("watch_stop"); d.textContent = "Closed"; }
    else { d.textContent = "Looking for your watch…"; d.textContent = await invoke("watch_start"); $("watPairRow").hidden = true; }
  } catch (err) { d.textContent = String(err); if (/^Pair the watch/.test(String(err))) $("watPairRow").hidden = false; }
  b.disabled = false; mirrorRefresh();
});
$("watPair").addEventListener("click", async () => {
  const b = $("watPair"); b.disabled = true; const d = $("watDesc");
  try { d.textContent = await invoke("watch_pair", { addr: $("watAddr").value, code: $("watCode").value }); $("watPairRow").hidden = true; $("watCode").value = ""; }
  catch (err) { d.textContent = String(err); }
  b.disabled = false;
});
async function mirrorRefresh() {
  loadPhoneApps();
  try { $("watShow").textContent = (await invoke("watch_running")) ? "Close watch" : "Show watch"; } catch {}
  let on = false; try { on = await invoke("mirror_running"); } catch {}
  $("mirShow").textContent = on ? "Close phone" : "Show phone";
}
$("mirShow").addEventListener("click", async () => {
  const b = $("mirShow"); b.disabled = true;
  try {
    if (await invoke("mirror_running")) { await invoke("mirror_stop"); $("mirDesc").textContent = "Closed"; }
    else { $("mirDesc").textContent = "Looking for your phone…"; $("mirDesc").textContent = await invoke("mirror_start"); }
  } catch (err) { $("mirDesc").textContent = String(err); }
  b.disabled = false; mirrorRefresh();
});
// ---- Recent on phone: one phone app per window (virtual display)
async function loadPhoneApps() {
  let p = { apps: [] }; try { p = await invoke("phone_apps"); } catch {}
  const g = $("phoneApps"); g.textContent = "";
  if (!p.apps.length) {
    const r = document.createElement("div"); r.className = "row";
    r.innerHTML = '<span class="row-text"><span class="row-desc">Nothing yet. The list appears once the phone is connected and Usage access is on for Aura Unlock.</span></span>';
    g.appendChild(r); return;
  }
  for (const a of p.apps) {
    const r = document.createElement("div"); r.className = "row";
    if (a.icon) { const im = document.createElement("img"); im.src = a.icon; im.alt = ""; im.width = 32; im.height = 32; im.style.cssText = "border-radius:8px;margin-right:12px;flex:none"; r.appendChild(im); }
    const t = document.createElement("span"); t.className = "row-text";
    const l = document.createElement("span"); l.className = "row-label"; l.textContent = a.label;
    const d = document.createElement("span"); d.className = "row-desc"; d.textContent = a.pkg;
    t.append(l, d);
    const b = document.createElement("button"); b.className = "btn"; b.type = "button"; b.textContent = "Open here";
    b.addEventListener("click", async () => {
      b.disabled = true; d.textContent = "Opening…";
      try { d.textContent = await invoke("phone_app_open", { pkg: a.pkg, label: a.label }); } catch (err) { d.textContent = String(err); }
      b.disabled = false;
    });
    const pin = document.createElement("button"); pin.className = "btn"; pin.type = "button"; pin.textContent = "Pin";
    pin.title = "Add to Start. Windows only lets you pin to the taskbar from there.";
    pin.addEventListener("click", async () => {
      pin.disabled = true;
      try { d.textContent = await invoke("phone_app_pin", { pkg: a.pkg, label: a.label }); } catch (err) { d.textContent = String(err); }
      pin.disabled = false;
    });
    r.append(t, pin, b); g.appendChild(r);
  }
}
$("mirRemote").addEventListener("click", async () => {
  const b = $("mirRemote"); b.disabled = true;
  try { $("mirRemDesc").textContent = await invoke("mirror_enable_remote"); } catch (err) { $("mirRemDesc").textContent = String(err); }
  b.disabled = false;
});
// ---- Laptop on phone (Sunshine host; the phone runs Moonlight)
function sunText(st) {
  if (st.store) return "Get the full edition: streaming this PC to your phone is an add-on that the Microsoft Store edition does not include. Download the full edition from the Aura website.";
  if (!st.installed) return "Not installed yet. Run tools\install-sunshine.ps1 once (one Windows permission click)";
  if (!st.credentials) return "Installed but not set up by Aura. Run tools\install-sunshine.ps1 again";
  if (!st.running) return "Off. Turn it on to control this PC from your phone";
  const p = st.paired == null ? "" : st.paired === 0 ? " No phone paired yet: use Pair phone below." : ` ${st.paired} phone${st.paired === 1 ? "" : "s"} paired.`;
  return (st.apiOk ? "On, ready for Moonlight." : "On, starting up…") + p;
}
async function sunRefresh() {
  let st = null; try { st = await invoke("sunshine_state"); } catch {}
  if (!st) return;
  $("sunToggle").textContent = st.running ? "Stop" : "Start";
  $("sunToggle").disabled = !st.installed;
  $("sunPair").disabled = !st.running || !st.apiOk;
  $("sunDesc").textContent = sunText(st);
}
$("sunToggle").addEventListener("click", async () => {
  const b = $("sunToggle"); b.disabled = true; $("sunDesc").textContent = "Working…";
  try { $("sunDesc").textContent = await invoke("sunshine_toggle"); } catch (err) { $("sunDesc").textContent = String(err); }
  setTimeout(sunRefresh, 1200);
});
$("sunPair").addEventListener("click", async () => {
  const b = $("sunPair"); b.disabled = true;
  try { $("sunPairDesc").textContent = await invoke("sunshine_pair", { pin: $("sunPin").value.trim() }); $("sunPin").value = ""; }
  catch (err) { $("sunPairDesc").textContent = String(err); }
  b.disabled = false; sunRefresh();
});
$("sunPin").addEventListener("input", (e) => { e.target.value = e.target.value.replace(/\D/g, "").slice(0, 4); });
$("sunPin").addEventListener("keydown", (e) => { if (e.key === "Enter") $("sunPair").click(); });
$("sunSend").addEventListener("click", async () => {
  try { $("sunSendDesc").textContent = await invoke("sunshine_send_info"); } catch (err) { $("sunSendDesc").textContent = String(err); }
});
$("sunApk").addEventListener("click", async () => {
  const b = $("sunApk"); b.disabled = true;
  try { $("sunApkDesc").textContent = await invoke("sunshine_send_moonlight"); } catch (err) { $("sunApkDesc").textContent = String(err); }
  b.disabled = false;
});
$("ringTest").addEventListener("click", async () => {
  try { await invoke("ring_pc_test"); } catch (err) { $("rTestDesc").textContent = String(err); }
});
document.querySelectorAll("[data-media]").forEach((b) => b.addEventListener("click", async () => {
  try { await invoke("media_local", { cmd: +b.dataset.media }); } catch (err) { $("rArtist").textContent = String(err); }
  setTimeout(loadRemote, 400);
}));

// ------------------------------------------------------------------ refresh loop (only while visible)
function refreshPage() {
  if (document.hidden) return;
  loadAura();
  if (page === "lock") loadLock();
  if (page === "handoff") loadHandoff();
  if (page === "remote") loadRemote();
}
setInterval(() => { if (!document.hidden && page !== "about" && !recording) refreshPage(); }, 4000);
window.addEventListener("focus", refreshPage);

async function reveal() {
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  try { await win.show(); await win.setFocus(); } catch {}
}

(async () => {
  go(location.hash.slice(1) || "unlock", false);
  await listen("navigate", (e) => go(String(e.payload)));
  await listen("status", (e) => { LS = e.payload; if (page === "lock") { renderHero(LS); refreshChips(); } });
  await listen("config", (e) => { cfg = e.payload; if (!lockDirty) fillForm(cfg); $("liveMode").checked = cfg.live_mode; });
  await Promise.all([loadAura(), loadLock()]);
  reveal();
  loadPaired();
})();
