// Continuity window. Plain JS, no build step, no CDN.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
};
const linkName = { Usb: "USB cable", Bluetooth: "Bluetooth", Wifi: "Wi-Fi", Tailscale: "Tailscale" };

window.showTab = (tab) => {
  for (const b of document.querySelectorAll(".tabs button")) b.setAttribute("aria-selected", String(b.dataset.tab === tab));
  for (const p of document.querySelectorAll(".panel")) p.hidden = p.id !== tab;
  if (tab === "clipboard") { loadClips(); $("q").focus(); }
};
for (const b of document.querySelectorAll(".tabs button")) b.onclick = () => window.showTab(b.dataset.tab);

function ago(ms) {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return Math.floor(s / 60) + " min ago";
  if (s < 86400) return Math.floor(s / 3600) + " h ago";
  return new Date(ms).toLocaleString();
}

function render(s) {
  $("state").dataset.key = s.key;
  $("state-label").textContent = s.label;
  $("phone-line").textContent = s.phone ? "Phone: " + s.phone.name : "No paired phone in KDE Connect";
  $("hotkey").textContent = s.hotkey;
  $("data-dir").textContent = s.data_dir;
  const links = $("links");
  links.replaceChildren();
  for (const l of s.facts.links) links.append(el("li", null, linkName[l] || l));
  if (!s.facts.links.length) links.append(el("li", null, "No link"));
  const f = $("findings");
  f.replaceChildren();
  if (!s.findings.length) f.append(el("li", "ok", "Nothing to fix."));
  for (const x of s.findings) {
    const li = el("li", x.severity);
    li.append(el("div", "t", x.title), el("div", "sub", x.why));
    if (x.steps.length) {
      const ol = el("ol");
      for (const st of x.steps) ol.append(el("li", null, st));
      li.append(ol);
    }
    if (x.fix.kind !== "steps" && x.fix_label) {
      const b = el("button", "act", x.fix_label);
      b.onclick = async () => {
        try { b.textContent = (await invoke("run_fix", { id: x.id })) || x.fix_label; }
        catch (e) { b.textContent = String(e); }
      };
      li.append(b);
    }
    f.append(li);
  }
  $("seen-line").textContent = s.last_seen;
  const ev = $("events");
  ev.replaceChildren();
  for (const e of s.events) {
    const via = e.via ? " (" + (linkName[e.via] || e.via) + ")" : "";
    ev.append(el("li", null, new Date(e.at_ms).toLocaleString() + "  " + (e.connected ? "connected" : "lost") + via));
  }
}

let qTimer;
async function loadClips() {
  const rows = await invoke("clip_search", { query: $("q").value });
  const ul = $("clips");
  ul.replaceChildren();
  if (!rows.length) ul.append(el("li", "ok", "Nothing here yet."));
  for (const r of rows) {
    const li = el("li", r.pinned ? "pinned" : "");
    const row = el("div", "row");
    const body = el("div", "txt");
    if (r.thumb) { const img = el("img"); img.src = r.thumb; img.alt = r.text; body.append(img); }
    else body.textContent = r.text;
    const copy = el("button", "act", "Copy");
    copy.onclick = async () => { await invoke("clip_copy", { id: r.id }); copy.textContent = "Copied"; };
    const pin = el("button", "ghost", r.pinned ? "Unpin" : "Pin");
    pin.onclick = async () => { await invoke("clip_pin", { id: r.id, on: !r.pinned }); loadClips(); };
    const del = el("button", "ghost", "Delete");
    del.onclick = async () => { await invoke("clip_delete", { id: r.id }); loadClips(); };
    row.append(body, copy, pin, del);
    li.append(row, el("div", "meta", (r.origin === "phone" ? "From phone" : "PC") + " - " + ago(r.at_ms)));
    ul.append(li);
  }
}
$("q").oninput = () => { clearTimeout(qTimer); qTimer = setTimeout(loadClips, 150); };
$("data-dir").onclick = (e) => { e.preventDefault(); invoke("open_data_dir"); };

listen("snapshot", (e) => render(e.payload));
listen("history", () => { if (!$("clipboard").hidden) loadClips(); });

(async () => {
  render(await invoke("get_snapshot"));
  window.showTab((location.hash || "#status").slice(1));
  requestAnimationFrame(() => window.__TAURI__.window.getCurrentWindow().show());
})();
