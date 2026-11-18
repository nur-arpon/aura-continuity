// AutoLock settings window. Plain JS, Tauri global API (withGlobalTauri).
// Same backend contract as before: get_config / get_status / list_paired /
// save_config / set_autostart / open_log_folder, events "status" + "config".
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);

let cfg = null;          // last config from the backend
let paired = null;       // null = still loading; [] = none; array of devices
let pairedError = null;  // string when list_paired failed
let selected = new Set();
let lastStatus = null;
let dirty = false;
const key = (d) => d.name;

// ------------------------------------------------------------------ helpers
const fmtAddr = (a) => a.toString(16).padStart(12, "0").toUpperCase().match(/../g).join(":");
const secs = (ms) => Math.floor(ms / 1000);
const KIND = {
  phone: ["Phone", "phone"], wearable: ["Watch", "watch"], audio: ["Audio", "headphones"],
  computer: ["Computer", "laptop"], peripheral: ["Accessory", "keyboard"], le: ["Bluetooth LE", "bluetooth"],
  other: ["Device", "bluetooth"], saved: ["Not paired right now", "bluetooth"],
};
function icon(name, cls = "ic") {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("class", cls); s.setAttribute("aria-hidden", "true");
  const u = document.createElementNS("http://www.w3.org/2000/svg", "use");
  u.setAttribute("href", `#i-${name}`); s.append(u); return s;
}
function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
function list(names) {
  if (names.length <= 1) return names.join("");
  return names.slice(0, -1).join(", ") + " and " + names.at(-1);
}

// ------------------------------------------------------------------ hero (live status)
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
      return { tone: "neutral", icon: "timer", title: "Waiting to see your devices", sub: "AutoLock starts watching once it has seen them nearby." };
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
        bluetooth_off: { tone: "warning", icon: "bt-off", title: "Bluetooth is off", sub: "AutoLock can't see your devices. Turn Bluetooth on in Windows quick settings." },
        paused: { tone: "neutral", icon: "pause", title: "Paused",
          sub: s.paused_left_secs != null ? `Back on in ${Math.max(1, Math.ceil(s.paused_left_secs / 60))} min. Resume early from the tray icon.` : "Resume from the tray icon." },
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
  if (k !== lastHeroKey) {   // a new kind of status: soft cross-fade (numbers ticking do not flash)
    for (const id of ["heroTitle", "heroSub"]) { const e = $(id); e.classList.remove("swap"); void e.offsetWidth; e.classList.add("swap"); }
    lastHeroKey = k;
  }
  $("heroTitle").textContent = h.title;
  $("heroSub").textContent = h.sub;
  const m = $("meter");
  m.hidden = h.meter == null;
  if (h.meter != null) m.firstElementChild.style.setProperty("--p", Math.min(1, h.meter).toFixed(3));

  const mode = $("mode");
  mode.textContent = s.live_mode ? "Live" : "Dry run";
  mode.dataset.tone = s.live_mode ? "warning" : "primary";
  mode.title = s.live_mode ? "Live: really locks. Switch to dry run from the tray icon menu."
                           : "Dry run: never locks. Switch to live from the tray icon menu.";
}

// ------------------------------------------------------------------ devices
function presenceChip(d, live) {
  const sel = selected.has(key(d));
  if (!sel) return null;
  if (!live) return el("span", "chip", "Save to watch");
  if (live.presence === "present") { const c = el("span", "chip", "Nearby"); c.dataset.tone = "success"; return c; }
  if (live.presence === "absent") { const c = el("span", "chip", "Away"); c.dataset.tone = "warning"; return c; }
  const c = el("span", "chip", "Checking"); c.dataset.tone = "neutral"; return c;
}

function emptyState({ tone, iconName, title, text, action, onAction }) {
  const box = el("div", "empty");
  if (tone) box.dataset.tone = tone;
  const ic = el("div", "empty-icon"); ic.append(icon(iconName)); box.append(ic);
  box.append(el("h3", null, title), el("p", null, text));
  if (action) {
    const b = el("button", "btn"); b.append(icon("refresh", "ic sm"), document.createTextNode(action));
    b.onclick = onAction; box.append(b);
  }
  return box;
}

function renderDevices() {
  const box = $("devices");
  if (paired === null) return;
  rows = new Map();
  if (pairedError && !paired.length) {
    box.replaceChildren(emptyState({ tone: "warning", iconName: "warning", title: "Couldn't read your Bluetooth devices",
      text: String(pairedError), action: "Try again", onAction: loadPaired }));
    return;
  }
  if (!paired.length) {
    box.replaceChildren(emptyState({ iconName: "bluetooth", title: "No paired devices yet",
      text: "Pair your phone or watch in Windows Settings › Bluetooth & devices, then check again.",
      action: "Check again", onAction: loadPaired }));
    return;
  }
  rows = new Map();
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

// Status ticks once a second: update only the chips, never rebuild the rows
// (rebuilding would steal keyboard focus and restart hover states).
let rows = new Map();
function refreshChips() {
  const live = new Map((lastStatus?.devices || []).map((d) => [d.name, d]));
  for (const r of rows.values()) {
    r.cb.checked = selected.has(key(r.d));
    const l = live.get(r.d.name);
    const watched = selected.has(key(r.d)) && l;
    const signal = !watched ? "" : l.rssi != null ? ` · signal ${String(l.rssi).replace("-", "−")} dBm`
      : l.connected ? " · connected" : "";
    r.desc.textContent = r.kindName + signal;
    const chip = presenceChip(r.d, l);
    const sig = chip ? chip.textContent + (chip.dataset.tone || "") : "";
    if (sig === r.last) continue;
    r.last = sig;
    r.slot.replaceChildren(...(chip ? [chip] : []));
  }
}

async function loadPaired() {
  pairedError = null;
  try { paired = await invoke("list_paired"); }
  catch (e) { paired = []; pairedError = e; }
  // Keep configured devices visible even if Windows did not list them this time.
  for (const d of cfg?.devices || []) if (!paired.some((p) => key(p) === key(d))) paired.push({ ...d, kind: "saved" });
  renderDevices();
  if (lastStatus) renderHero(lastStatus);
}

// ------------------------------------------------------------------ form
function setRangeFill() {
  const r = $("rssi");
  const pct = ((r.value - r.min) / (r.max - r.min)) * 100;
  r.style.setProperty("--fill", pct + "%");
  $("rssival").textContent = `${String(r.value).replace("-", "−")} dBm`;
  $("rssirow").classList.toggle("is-disabled", !$("userssi").checked);
  $("rssi").disabled = !$("userssi").checked;
}

function fillForm(c) {
  $("absent").value = c.absent_secs;
  const r = document.querySelector(`input[name=when][value=${c.lock_when}]`);
  if (r) r.checked = true;
  $("userssi").checked = c.use_rssi;
  $("rssi").value = c.rssi_threshold;
  $("fullscreen").checked = c.block_on_fullscreen;
  $("ping").value = c.ping_interval_secs;
  selected = new Set(c.devices.map(key));
  setRangeFill();
}

let noteTimer = 0;
function note(text, tone, sticky) {
  const n = $("saved");
  clearTimeout(noteTimer);
  n.textContent = text; n.dataset.tone = tone || "";
  n.classList.toggle("show", !!text);
  if (!sticky && text) noteTimer = setTimeout(() => n.classList.remove("show"), 2400);
}
function markDirty() {
  dirty = true;
  $("save").disabled = false; $("revert").hidden = false;
  note("Unsaved changes", "", true);
}
function markClean() {
  dirty = false;
  $("save").disabled = true; $("revert").hidden = true;
}

function clampInput(input) {
  const v = Math.round(+input.value);
  const lo = +input.min, hi = +input.max;
  const c = Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : lo;
  if (String(c) !== input.value) input.value = c;
}
for (const id of ["absent", "ping"]) {
  $(id).addEventListener("input", markDirty);
  $(id).addEventListener("blur", () => clampInput($(id)));
}
for (const id of ["userssi", "rssi", "fullscreen"]) $(id).addEventListener("input", () => { setRangeFill(); markDirty(); });
document.querySelectorAll("input[name=when]").forEach((r) => r.addEventListener("change", markDirty));

async function save() {
  if (!dirty) return;
  clampInput($("absent")); clampInput($("ping"));
  const devices = (paired || []).filter((d) => selected.has(key(d))).map((d) => ({ name: d.name, classic: d.classic, le: d.le }));
  const next = {
    ...cfg,
    devices,
    absent_secs: +$("absent").value,
    lock_when: document.querySelector("input[name=when]:checked").value,
    use_rssi: $("userssi").checked,
    rssi_threshold: +$("rssi").value,
    block_on_fullscreen: $("fullscreen").checked,
    ping_interval_secs: +$("ping").value,
  };
  $("save").disabled = true;
  try {
    await invoke("save_config", { config: next });
    cfg = await invoke("get_config");
    fillForm(cfg);
    markClean();
    note("Saved", "success");
    refreshChips();
  } catch (e) {
    $("save").disabled = false;
    note(`Couldn't save: ${e}`, "warning", true);
  }
}
$("save").onclick = save;
$("revert").onclick = () => { fillForm(cfg); markClean(); note(""); refreshChips(); };
document.addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") { e.preventDefault(); save(); }
  // Esc closes the window (the app keeps running in the tray); not while a menu is open or edits are unsaved.
  if (e.key === "Escape" && !e.defaultPrevented && !dirty && document.activeElement?.tagName !== "SELECT") {
    try { window.__TAURI__.window.getCurrentWindow().close(); } catch {}
  }
});

$("autostart").onchange = async (e) => {
  const want = e.target.checked;
  try {
    e.target.checked = await invoke("set_autostart", { on: want });
    note(e.target.checked ? "Starts with Windows" : "Won't start with Windows", "success");
  } catch (err) {
    e.target.checked = !want;
    note(`Couldn't change startup: ${err}`, "warning", true);
  }
};
$("logs").onclick = () => invoke("open_log_folder");

// ------------------------------------------------------------------ status
function renderStatus(s) {
  lastStatus = s;
  renderHero(s);
  $("autostart").checked = s.autostart;
  $("logpath").textContent = s.log_path || "";
  $("logpath").title = s.log_path || "";
  refreshChips();
}

async function reveal() {
  // Show after first paint; the window is created hidden (no white flash).
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  try {
    const w = window.__TAURI__.window.getCurrentWindow();
    await w.show(); await w.setFocus();
  } catch {}
}

(async () => {
  try {
    cfg = await invoke("get_config");
    fillForm(cfg);
    renderStatus(await invoke("get_status"));
  } catch (e) {
    $("heroTitle").textContent = "Couldn't reach AutoLock";
    $("heroSub").textContent = String(e);
    $("hero").dataset.tone = "warning";
  }
  reveal();
  await loadPaired();
  await listen("status", (e) => renderStatus(e.payload));
  await listen("config", (e) => { if (!dirty) { cfg = e.payload; fillForm(cfg); refreshChips(); } });
})();
