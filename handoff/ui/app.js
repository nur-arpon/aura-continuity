// Handoff settings window. Plain JS, no build step; talks to Rust via window.__TAURI__.
// Backend contract unchanged: get_state / save_settings / open_data_dir.
const { invoke } = window.__TAURI__.core;
const win = window.__TAURI__.window.getCurrentWindow();
const $ = (id) => document.getElementById(id);

let current = null;
let noteTimer = 0;

function note(text, tone, sticky) {
  const n = $("saved");
  clearTimeout(noteTimer);
  n.textContent = text; n.dataset.tone = tone || "";
  n.classList.toggle("show", !!text);
  if (!sticky && text) noteTimer = setTimeout(() => n.classList.remove("show"), 2000);
}

// ------------------------------------------------------------------ status card
let lastHeroKey = "";
function renderHero(state) {
  const hero = $("hero");
  const q = state.queueLen || 0;
  let tone, icon, title, sub;
  if (state.peer) {
    tone = q ? "warning" : "success"; icon = q ? "clock" : "phone";
    title = `${state.peer.name} is ready`;
    sub = q ? `${q} waiting to send. Going now.` : state.incomingConnected
      ? `Connected through ${state.backend}. Sending and receiving.`
      : `Connected through ${state.backend}. Receiving is still connecting.`;
  } else if (state.devicesError) {
    tone = "warning"; icon = "warning"; title = `${state.backend} isn't answering`;
    sub = state.backend === "Aura Link" ? "Start Aura on this PC, then come back here." : "Open KDE Connect on this PC, then come back here.";
  } else if (!state.devices?.length) {
    tone = "neutral"; icon = "phone"; title = "No phone paired yet";
    sub = state.backend === "Aura Link" ? "Pair your phone in the Aura phone app first." : "Pair your phone in KDE Connect first.";
  } else {
    tone = q ? "warning" : "neutral"; icon = q ? "clock" : "phone"; title = "Phone not reachable";
    sub = q ? `${q} waiting. They send by themselves when the phone is back.`
            : "Anything you send now waits here until the phone is back.";
  }
  hero.dataset.tone = tone;
  $("heroIcon").setAttribute("href", `#i-${icon}`);
  const k = tone + icon + title;
  if (k !== lastHeroKey) {
    for (const id of ["peer", "backend"]) { const e = $(id); e.classList.remove("swap"); void e.offsetWidth; e.classList.add("swap"); }
    lastHeroKey = k;
  }
  $("peer").textContent = title;
  $("backend").textContent = sub;
}

// ------------------------------------------------------------------ shortcut recorder
const hk = $("hotkey");
let recording = false;
function showKeys(accel) {
  hk.replaceChildren();
  accel.split("+").forEach((k, i) => {
    if (i) { const p = document.createElement("span"); p.className = "plus"; p.textContent = "+"; hk.append(p); }
    const kb = document.createElement("kbd"); kb.textContent = k === "Super" ? "Win" : k; hk.append(kb);
  });
  hk.setAttribute("aria-label", `Send shortcut ${accel}. Press Enter to change it.`);
}
function showHint(text, nudge) {
  hk.replaceChildren();
  const s = document.createElement("span"); s.className = "hint"; s.textContent = text; hk.append(s);
  hk.classList.toggle("nudge", !!nudge);
}
function startRecording() {
  recording = true; hk.classList.add("recording"); showHint("Press the new keys…");
  hk.setAttribute("aria-label", "Recording. Press the new shortcut, or Esc to cancel.");
}
function stopRecording() {
  recording = false; hk.classList.remove("recording", "nudge");
  if (current) showKeys(current.hotkey);
}
hk.addEventListener("click", () => (recording ? stopRecording() : startRecording()));
hk.addEventListener("blur", () => recording && stopRecording());

const KEY_NAMES = { Space: "Space", Enter: "Enter", Backquote: "`", Minus: "-", Equal: "=", Comma: ",", Period: ".", Slash: "/" };
function keyName(code) {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  if (/^F\d{1,2}$/.test(code)) return code;
  return KEY_NAMES[code] ?? null;
}
hk.addEventListener("keydown", (e) => {
  if (!recording) return;   // Enter / Space fall through to "click" and start recording
  e.preventDefault(); e.stopPropagation();
  if (e.key === "Escape") { stopRecording(); return; }
  const k = keyName(e.code);
  if (!k) return; // modifier alone - keep waiting
  const mods = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  if (!mods.some((m) => m !== "Shift")) { showHint("Add Ctrl, Alt or Win", true); return; }
  const accel = [...mods, k].join("+");
  recording = false; hk.classList.remove("recording", "nudge");
  save({ hotkey: accel });
});

// ------------------------------------------------------------------ render + save
function render(state) {
  current = state.settings;
  renderHero(state);
  if (!recording) showKeys(current.hotkey);
  $("openInBrave").checked = current.openInBrave;
  $("youtubeTimestamp").checked = current.youtubeTimestamp;
  $("autostart").checked = current.autostart;
  $("suggest").checked = current.suggest;
  $("clipboardSync").checked = current.clipboardSync;
  $("deviceD").textContent = `Paired through ${state.backend}`;
  $("version").textContent = `Handoff ${state.version}`;

  const sel = $("device");
  if (document.activeElement !== sel) {
    sel.replaceChildren();
    sel.add(new Option("Any paired phone", ""));
    for (const d of state.devices) sel.add(new Option(d.name, d.id));
    if (state.devicesError) sel.add(new Option(`${state.backend} isn't answering`, "__err", false, false));
    if (current.deviceId && !state.devices.some((d) => d.id === current.deviceId))
      sel.add(new Option("Saved phone (not found now)", current.deviceId));
    sel.value = current.deviceId ?? "";
  }
}

async function refresh() {
  try {
    render(await invoke("get_state"));
  } catch (e) {
    $("hero").dataset.tone = "warning";
    $("heroIcon").setAttribute("href", "#i-warning");
    $("peer").textContent = "Couldn't read Handoff's state";
    $("backend").textContent = String(e);
  }
}

async function save(patch) {
  const next = { ...current, ...patch };
  try {
    current = await invoke("save_settings", { new: next });
    $("hotkeyError").hidden = true;
    note("Saved", "success");
  } catch (e) {
    $("hotkeyError").textContent = String(e);
    $("hotkeyError").hidden = false;
    note("");
  }
  if (!recording) showKeys(current.hotkey);
  $("autostart").checked = current.autostart;
}

$("device").addEventListener("change", (e) => {
  if (e.target.value === "__err") { e.target.value = current.deviceId ?? ""; return; }
  save({ deviceId: e.target.value || null });
});
for (const id of ["openInBrave", "youtubeTimestamp", "autostart", "suggest", "clipboardSync"]) {
  $(id).addEventListener("change", (e) => save({ [id]: e.target.checked }));
}
$("openLogs").addEventListener("click", () => invoke("open_data_dir"));

// Files dropped on the window go to the phone (a tray icon cannot take drops on Windows).
window.__TAURI__.webview.getCurrentWebview().onDragDropEvent(async (e) => {
  const d = $("drop");
  if (e.payload.type === "over" || e.payload.type === "enter") d.classList.add("hot");
  else d.classList.remove("hot");
  if (e.payload.type === "drop" && e.payload.paths?.length) {
    try {
      const n = await invoke("send_paths", { paths: e.payload.paths });
      $("dropD").textContent = n ? `Sending ${n} file${n > 1 ? "s" : ""}\u2026` : "Only files can be sent (not folders)";
    } catch (err) { $("dropD").textContent = String(err); }
  }
});
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !recording) win.close(); });

// Show only after the first real paint (no white flash), then load live state.
requestAnimationFrame(() => requestAnimationFrame(async () => {
  await win.show();
  await win.setFocus();
  refresh();
}));
window.addEventListener("focus", refresh);
