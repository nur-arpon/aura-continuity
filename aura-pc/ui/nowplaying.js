// "Now playing everywhere" page: every session on this laptop and the phone, with transport + seek, and the
// phone-notification mirror switches. Loaded after app.js. The laptop session is read on demand (and every 3 s
// while this page is open and visible); the phone pushes its list by itself ("np" event).
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = (id) => document.getElementById(id);
  const pageEl = $("page-playing");
  if (!pageEl) return;
  let S = { laptop: null, phone: [], phoneLinked: false, phoneAgeMs: 0 };
  let fetchedAt = 0, timer = null, tick = null, busy = false;

  const visible = () => !pageEl.hidden && !document.hidden;
  const fmt = (ms) => { const s = Math.max(0, Math.floor(ms / 1000)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };
  const posOf = (x) => { const p = x.playing ? x.pos + (Date.now() - fetchedAt) : x.pos; return x.dur > 0 ? Math.min(p, x.dur) : p; };

  function el(tag, cls, text) { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; }

  function btn(label, svg, cmd, x, primary) {
    const b = el("button", "btn" + (primary ? " btn-primary" : ""));
    b.type = "button"; b.setAttribute("aria-label", label);
    b.innerHTML = `<svg class="ic sm" aria-hidden="true"><use href="#i-${svg}"/></svg>`;
    b.addEventListener("click", async () => {
      try { await invoke("np_cmd", { device: x.device, id: x.id, cmd, pos: null }); setTimeout(refresh, x.device === "laptop" ? 350 : 900); }
      catch (e) { $("npMeta").textContent = String(e); }
    });
    return b;
  }

  function card(x, stale) {
    const c = el("div", "card np-card" + (stale ? " np-stale" : ""));
    c.dataset.key = `${x.device}:${x.id}`;
    const top = el("div", "card-top");
    const art = el("span", "np-art");
    if (x.art) art.style.backgroundImage = `url("${x.art}")`; else art.innerHTML = `<svg class="ic" aria-hidden="true"><use href="#i-${x.device === "laptop" ? "laptop" : "phone"}"/></svg>`;
    const txt = el("span", "row-text");
    txt.append(el("span", "row-label", x.title || "Unknown"), el("span", "row-desc", `${x.device === "laptop" ? "Laptop" : "Phone"} · ${x.app}${x.artist ? " · " + x.artist : ""}`));
    top.append(art, txt);
    const bar = el("div", "np-bar"); bar.setAttribute("role", "slider"); bar.setAttribute("aria-label", "Position");
    const fill = el("div", "np-fill"); bar.append(fill);
    const times = el("div", "np-times"); const t0 = el("span", null, "0:00"), t1 = el("span", null, x.dur ? fmt(x.dur) : "");
    times.append(t0, t1);
    bar.addEventListener("click", async (e) => {
      if (!x.dur) return;
      const r = bar.getBoundingClientRect();
      const pos = Math.round(((e.clientX - r.left) / r.width) * x.dur);
      try { await invoke("np_cmd", { device: x.device, id: x.id, cmd: "seek", pos }); setTimeout(refresh, 600); } catch (err) { $("npMeta").textContent = String(err); }
    });
    const actions = el("div", "card-actions");
    actions.append(btn("Previous", "prev", "prev", x), btn(x.playing ? "Pause" : "Play", x.playing ? "pause" : "playtri", "toggle", x, true), btn("Next", "next", "next", x));
    c.append(top, bar, times, actions);
    c._u = () => { const p = posOf(x); fill.style.width = x.dur ? `${Math.min(100, (p / x.dur) * 100)}%` : "0%"; t0.textContent = fmt(p); };
    c._u();
    return c;
  }

  function render() {
    const list = $("npList");
    const items = [];
    if (S.laptop) items.push([S.laptop, false]);
    // a phone list older than 15 minutes is probably stale (the phone only talks when something changes): grey it
    for (const p of S.phone) items.push([p, !S.phoneLinked]);
    list.replaceChildren(...items.map(([x, stale]) => card(x, stale)));
    if (!items.length) list.append(el("p", "section-note", "Nothing is playing on this laptop or your phone."));
    $("npMeta").textContent = S.phoneLinked ? "Phone connected" : "Phone not connected";
  }

  async function refresh() {
    if (busy) return; busy = true;
    try { S = await invoke("np_state"); fetchedAt = Date.now(); render(); } catch (e) { $("npMeta").textContent = String(e); }
    busy = false;
  }

  function loop() {
    clearInterval(timer); clearInterval(tick); timer = tick = null;
    if (!visible()) return;
    refresh();
    timer = setInterval(() => { if (visible()) refresh(); }, 3000);
    tick = setInterval(() => { if (visible()) document.querySelectorAll(".np-card").forEach((c) => c._u && c._u()); }, 1000);
    loadNotif();
  }

  // ---- notification mirror
  async function loadNotif() {
    try {
      const n = await invoke("notif_state");
      $("nmOn").checked = n.enabled;
      const g = $("nmApps");
      g.replaceChildren(...n.apps.map((a) => {
        const row = el("label", "row clickable");
        const ic = el("span", "np-appicon"); if (a.icon) ic.style.backgroundImage = `url("${a.icon}")`;
        const t = el("span", "row-text"); t.append(el("span", "row-label", a.name), el("span", "row-desc", a.pkg));
        const sw = el("input", "switch"); sw.type = "checkbox"; sw.setAttribute("role", "switch"); sw.checked = !a.muted;
        sw.addEventListener("change", () => invoke("notif_set_app", { pkg: a.pkg, muted: !sw.checked }));
        row.append(ic, t, sw);
        return row;
      }));
      g.hidden = n.apps.length === 0;
    } catch (e) { /* page still works without it */ }
  }
  $("nmOn").addEventListener("change", (e) => invoke("notif_set_enabled", { on: e.target.checked }));
  $("nmTest").addEventListener("click", async () => { try { await invoke("notif_test"); } catch (e) { $("nmNote").textContent = String(e); } });

  new MutationObserver(loop).observe(pageEl, { attributes: true, attributeFilter: ["hidden"] });
  document.addEventListener("visibilitychange", loop);
  listen("np", () => { if (visible()) refresh(); });
  loop();
})();
