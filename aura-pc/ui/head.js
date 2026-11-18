// Chat-head tree (laptop): head -> phone's recent apps (real icons, on rings) -> a phone browser's recent pages.
// Springs come from Motion (vendor/motion.js). Window: small around the head, or the whole work area while open / dragging.
// The head stays put until it is thrown on the X; it is clamped to the work area of its monitor (DPI-aware: the backend
// positions the window in physical px) and can be dropped onto another monitor.
const { invoke } = window.__TAURI__.core;
const { animate } = window.Motion;
const $ = (id) => document.getElementById(id);
const stage = $("stage"), head = $("head"), xBtn = $("x"), toast = $("toast");
const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
const HEAD = 56, PAD = 14, BUB = 52, EDGE = 4, GAP = 78, SPAN = 2.8;
const SPRING = { type: "spring", stiffness: 420, damping: 26, mass: 0.8 };
const SOFT = { type: "spring", stiffness: 320, damping: 32 };
const note = (m) => { try { invoke("head_log", { msg: String(m) }); } catch {} };
addEventListener("error", (e) => note("error: " + e.message + " @" + e.lineno));
addEventListener("unhandledrejection", (e) => note("rejected: " + (e.reason && e.reason.message || e.reason)));
const spring = (o = {}) => (reduce ? { duration: 0.1 } : { ...SPRING, ...o });

let sw = 1920, sh = 1040, mon = { mx: null, my: null, scale: 1 }, hx = 0, hy = 0; // work area (logical px) and the head's top-left in it
let full = false, level = 0, data = { apps: [], pages: [] }, busy = false, acting = false;
const savedPos = () => { try { return JSON.parse(localStorage.getItem("headPos")); } catch { return null; } };
const savePos = () => { try { localStorage.setItem("headPos", JSON.stringify({ y: hy / sh, right: hx > sw / 2, mx: mon.mx, my: mon.my })); } catch {} };

function layoutHead() {
  head.style.left = (full ? hx : PAD) + "px";
  head.style.top = (full ? hy : PAD) + "px";
  stage.style.setProperty("--hx", (hx + HEAD / 2) + "px");
  stage.style.setProperty("--hy", (hy + HEAD / 2) + "px");
}
async function place(wantFull) {
  full = wantFull;
  await invoke("head_place", { full: wantFull, x: (hx - PAD) * k(), y: (hy - PAD) * k(), size: (HEAD + 2 * PAD) * k() });
  layoutHead();
}
const clampX = (x) => Math.max(0, Math.min(sw - HEAD, x));
const clampY = (y) => Math.max(8, Math.min(sh - HEAD - 8, y));
const inward = () => (hx + HEAD / 2 > sw / 2 ? -1 : 1);
// The webview can render at a different scale than the monitor (dpr 1.635 on a 1.5 monitor: a 1707-px-wide work area was
// only 1566 css px). Page units are css px; the backend speaks monitor-logical px: k converts page -> backend.
const k = () => devicePixelRatio / (mon.scale || 1);
function adoptScreen(s) { mon = { mx: s.mx, my: s.my, scale: s.scale, rw: s.w, rh: s.h, dpr: devicePixelRatio }; sw = s.w / k(); sh = s.h / k(); }

let media = [];
async function refresh() { try { data = await invoke("head_data"); } catch {} try { media = await invoke("head_media"); } catch {} head.classList.toggle("playing", media.some((s) => s.playing)); }

// Now playing chip (shown while the head is fanned out): the playing session (phone first), prev / play-pause / next; hold it to open the Playing page
const npEl = $("np");
const npSession = () => media[0];
function renderNp() {
  const s = npSession();
  npEl.style.display = s && level ? "flex" : "none";
  if (!s) return;
  $("npT").textContent = s.title;
  $("npA").textContent = (s.device === "laptop" ? "This laptop" : s.app) + (s.artist ? " · " + s.artist : "");
  $("npArt").style.backgroundImage = s.art ? `url(${s.art})` : "";
  $("npPP").textContent = s.playing ? "⏸" : "▶";
}
async function npCmd(cmd) {
  const s = npSession(); if (!s) return;
  try { await invoke("np_cmd", { device: s.device, id: s.id, cmd, pos: null }); } catch (e) { say(String(e).slice(0, 90), true); return; }
  if (cmd === "toggle") { s.playing = !s.playing; renderNp(); }
  setTimeout(async () => { await refresh(); renderNp(); }, 900);
}
for (const [id, cmd] of [["npPrev", "prev"], ["npPP", "toggle"], ["npNext", "next"]]) $(id).addEventListener("click", (e) => { e.stopPropagation(); npCmd(cmd); });
let npHold = null;
npEl.addEventListener("pointerdown", (e) => { if (e.target.closest("button")) return; npHold = setTimeout(async () => { npHold = null; try { await invoke("head_open_playing"); } catch {} collapse(); }, 550); });
for (const ev of ["pointerup", "pointerleave", "pointercancel"]) npEl.addEventListener(ev, () => { clearTimeout(npHold); npHold = null; });
npEl.addEventListener("click", (e) => e.stopPropagation());

function letterColor(s) { let h = 0; for (const c of s) h = (h * 31 + c.charCodeAt(0)) >>> 0; return `hsl(${h % 360} 55% 42%)`; }
function mk(cls, label, iconUri, letter, unread) {
  const b = document.createElement("div");
  b.className = "b " + cls;
  if (iconUri) { const i = document.createElement("img"); i.className = "ico"; i.src = iconUri; b.appendChild(i); }
  else { const l = document.createElement("div"); l.className = "ltr"; l.style.background = letterColor(letter || label); l.textContent = (letter || label || "?").trim().charAt(0).toUpperCase(); b.appendChild(l); }
  if (unread > 0) { const n = document.createElement("div"); n.className = "badge"; n.textContent = unread > 99 ? "99+" : String(unread); b.appendChild(n); }
  const lab = document.createElement("div"); lab.className = "lab"; lab.textContent = label; b.appendChild(lab);
  stage.appendChild(b);
  return b;
}

// Bubbles on rings around (cx, cy), the fan pointing towards the middle of the screen. Ring 1 holds as many as fit a
// GAP apart, the rest go to a bigger ring, and so on.
function ringPlan(n, r0, dr) {
  const plan = []; let left = n, r = r0;
  while (left > 0) {
    const cap = Math.max(1, Math.floor(SPAN / (GAP / r)) + 1), c = Math.min(left, cap);
    plan.push({ r, count: c }); left -= c; r += dr;
  }
  return plan;
}
function arcPoints(cx, cy, n, r0) {
  // towards the screen's middle: sideways, tilted up/down when the head is near the top or bottom edge
  const vy = Math.max(-1, Math.min(1, ((sh / 2 - cy) / sh) * 2.4)) * 0.9;
  const base = Math.atan2(vy, inward());
  const out = [];
  for (const { r, count } of ringPlan(n, r0, GAP + 18)) {
    const step = count > 1 ? Math.min(1.15 * GAP / r, SPAN / (count - 1)) : 0;
    for (let i = 0; i < count; i++) {
      const a = base + (i - (count - 1) / 2) * step;
      out.push([Math.max(8, Math.min(sw - BUB - 8, cx + r * Math.cos(a) - BUB / 2)), Math.max(8, Math.min(sh - BUB - 30, cy + r * Math.sin(a) - BUB / 2))]);
    }
  }
  return out;
}
function fanOut(els, cx, cy, R) {
  const pts = arcPoints(cx, cy, els.length, R);
  els.forEach((b, i) => {
    b.style.left = (cx - BUB / 2) + "px"; b.style.top = (cy - BUB / 2) + "px";
    animate(b, { x: [0, pts[i][0] - (cx - BUB / 2)], y: [0, pts[i][1] - (cy - BUB / 2)], scale: [0.2, 1], opacity: [0, 1] },
      { ...spring(), delay: reduce ? 0 : i * 0.035, opacity: { duration: 0.12, delay: i * 0.035 } });
  });
}
function clearBubbles(list, towards) {
  list.forEach((b, i) => {
    b.style.pointerEvents = "none";
    const a = animate(b, { opacity: 0, scale: 0.3, ...(towards ? { x: towards[0] - parseFloat(b.style.left), y: towards[1] - parseFloat(b.style.top) } : {}) }, { ...spring({ stiffness: 520, damping: 34 }), delay: i * 0.015 });
    a.then(() => b.remove());
  });
}

function say(t, bad) {
  toast.textContent = t; toast.style.display = "block"; toast.style.background = bad ? "#8a1f14" : "";
  clearTimeout(say.t); say.t = setTimeout(() => (toast.style.display = "none"), 2600);
}

let l1 = [], l2 = [], l1Open = null;
async function expand() {
  if (busy || level) return; busy = true; acting = false;
  await refresh();
  if (!data.apps.length && !media.length) { busy = false; return; }
  await place(true);
  stage.classList.add("open");
  level = 1; renderNp();
  const cx = hx + HEAD / 2, cy = hy + HEAD / 2;
  // every bubble carries its own app object: what is tapped is what is opened, however the list moves meanwhile
  l1 = data.apps.map((a) => { const b = mk("app", a.label, a.icon, a.label, a.unread); b._app = a; b.addEventListener("click", (e) => { e.stopPropagation(); onApp(b); }); return b; });
  fanOut(l1, cx, cy, 128);
  busy = false;
}
async function collapse() {
  if (busy || !level) return; busy = true;
  const to = [hx + HEAD / 2 - BUB / 2, hy + HEAD / 2 - BUB / 2];
  clearBubbles([...l2, ...l1], to); l1 = []; l2 = []; l1Open = null; level = 0;
  stage.classList.remove("open"); renderNp();
  await new Promise((r) => setTimeout(r, reduce ? 120 : 260));
  await place(false);
  busy = false;
}
function backToApps() {
  clearBubbles(l2, l1Open ? [parseFloat(l1Open.style.left) + (l1Open._dx || 0), parseFloat(l1Open.style.top) + (l1Open._dy || 0)] : null); l2 = []; level = 1;
  l1.forEach((b) => { b.style.pointerEvents = ""; animate(b, { opacity: 1, scale: 1 }, spring()); });
  l1Open = null;
}
async function onApp(b) {
  if (level !== 1 || acting) return;
  const a = b._app;
  const mine = data.pages.filter((p) => p.app === a.pkg), pages = mine.length ? mine : data.pages; // this browser's pages (all, if the phone does not say)
  if (a.browser && pages.length) {
    level = 2; l1Open = b;
    const r = b.getBoundingClientRect();
    b._dx = r.left - parseFloat(b.style.left); b._dy = r.top - parseFloat(b.style.top);
    l1.forEach((o) => { if (o !== b) { o.style.pointerEvents = "none"; animate(o, { opacity: 0.3, scale: 0.8 }, spring()); } });
    const cx = r.left + BUB / 2, cy = r.top + BUB / 2;
    l2 = pages.map((p) => { const x = mk("page", p.title, null, p.host || p.title); x._page = p; x.addEventListener("click", (e) => { e.stopPropagation(); onPage(x); }); return x; });
    fanOut(l2, cx, cy, 124);
    return;
  }
  acting = true; // one tap = one action
  note("tap app " + a.pkg);
  say("Opening " + a.label + " on a window of its own...");
  collapse();
  try { await invoke("head_open_app", { pkg: a.pkg, label: a.label }); }
  catch (e) { note("open app failed: " + e); say(String(e).slice(0, 90), true); }
}
async function onPage(b) {
  if (level !== 2 || acting) return;
  acting = true;
  say("Opening " + b._page.title.slice(0, 40));
  collapse();
  try { await invoke("head_open_page", { url: b._page.url }); }
  catch (e) { say(String(e).slice(0, 90), true); }
}

// tap outside = one level back; Escape too (when the window has focus)
stage.addEventListener("pointerdown", (e) => {
  if (e.target !== stage || !level) return;
  if (level === 2) backToApps(); else collapse();
});
addEventListener("keydown", (e) => { if (e.key === "Escape") { if (level === 2) backToApps(); else collapse(); } });

// head: tap toggles, drag moves along the screen and can be thrown on the X to dismiss it
let drag = null;
head.addEventListener("pointerdown", async (e) => {
  note("pointerdown busy=" + busy + " drag=" + !!drag + " full=" + full + " level=" + level);
  if (e.button !== 0 || busy || drag) return;
  try { head.setPointerCapture(e.pointerId); } catch {}
  const wasFull = full;
  // screen coordinates: they do not change when the window grows under the finger, so no movement is lost
  drag = { id: e.pointerId, moved: false, sx: e.screenX, sy: e.screenY, hx0: hx, hy0: hy, rx: hx, ry: hy, ready: wasFull };
  if (!wasFull) { await place(true); }
  if (drag) drag.ready = true;
});
// the X sits where the page really is (innerWidth/innerHeight), 90 px above its bottom: that is the visible work area
const xPos = () => ({ cx: innerWidth / 2, cy: innerHeight - 90 });
const overX = () => { const p = xPos(); return Math.hypot(hx + HEAD / 2 - p.cx, hy + HEAD / 2 - p.cy) < 80; };
addEventListener("pointermove", (e) => {
  if (!drag || !drag.ready || e.pointerId !== drag.id) return;
  const dx = e.screenX - drag.sx, dy = e.screenY - drag.sy;
  if (!drag.moved && Math.hypot(dx, dy) < 6) return;
  if (!drag.moved) {
    drag.moved = true; head.classList.add("drag");
    if (level) { clearBubbles([...l2, ...l1]); l1 = []; l2 = []; l1Open = null; level = 0; stage.classList.remove("open"); }
    const xp = xPos(); xBtn.style.left = (xp.cx - 32) + "px"; xBtn.style.top = (xp.cy - 32) + "px"; xBtn.style.display = "grid";
    note("drag start: inner " + innerWidth + "x" + innerHeight + " work " + Math.round(sw) + "x" + Math.round(sh) + " dpr " + devicePixelRatio);
    animate(xBtn, { opacity: [0, 1], scale: [0.6, 1] }, spring());
  }
  drag.rx = drag.hx0 + dx; drag.ry = drag.hy0 + dy; // where the pointer wants it (may be past the edge: another monitor)
  hx = clampX(drag.rx); hy = Math.max(0, Math.min(sh - HEAD, drag.ry));
  layoutHead();
  const near = overX();
  xBtn.classList.toggle("hot", near);
  animate(xBtn, { scale: near ? 1.25 : 1 }, { duration: 0.08 });
});
async function endDrag(e) {
  if (!drag || (e.pointerId !== undefined && e.pointerId !== drag.id)) return;
  const d = drag; drag = null; head.classList.remove("drag");
  try { head.releasePointerCapture(d.id); } catch {}
  if (!d.moved) { // a tap
    note("tap head level=" + level + " apps=" + data.apps.length);
    if (level) await collapse(); else await expand();
    return;
  }
  const over = overX();
  note("drop over=" + over + " at " + Math.round(hx) + "," + Math.round(hy) + " screen " + Math.round(sw) + "x" + Math.round(sh));
  animate(xBtn, { opacity: 0, scale: 0.6 }, { duration: 0.15 }).then(() => (xBtn.style.display = "none"));
  xBtn.classList.remove("hot");
  if (over) {
    await animate(head, { scale: 0, opacity: 0 }, spring({ stiffness: 500, damping: 40 }));
    await invoke("head_dismiss");
    hx = sw - HEAD - EDGE; hy = clampY(hy); savePos();
    await place(false); head.style.transform = ""; head.style.opacity = ""; return;
  }
  // dropped past the edge of this monitor: the monitor beyond becomes the head's
  const cx = d.rx + HEAD / 2, cy = d.ry + HEAD / 2;
  if (cx < 0 || cx > sw || cy < 0 || cy > sh) {
    try {
      const r = await invoke("head_retarget", { cx: cx * k(), cy: cy * k() });
      if (r.screen.mx !== mon.mx || r.screen.my !== mon.my) {
        adoptScreen(r.screen);
        hx = clampX(r.cx / k() - HEAD / 2); hy = clampY(r.cy / k() - HEAD / 2);
        await place(true);
      }
    } catch {}
  }
  // snap to the nearest edge, fully on screen
  const right = hx + HEAD / 2 > sw / 2;
  const tx = right ? sw - HEAD - EDGE : EDGE, ty = clampY(hy);
  await animate(head, { left: tx + "px", top: ty + "px" }, reduce ? { duration: 0.05 } : SOFT);
  hx = tx; hy = ty; savePos();
  await place(false);
}
addEventListener("pointerup", endDrag);
addEventListener("pointercancel", endDrag);

// keep the data fresh, and follow the screen: a monitor unplugged or a resolution / scaling change must not strand the head
async function watchScreen() {
  if (level || drag || busy) return;
  try {
    const s = await invoke("head_screen", { mx: null, my: null });
    if (level || drag || busy) return; // the owner started something while we waited
    if (s.w !== mon.rw || s.h !== mon.rh || s.mx !== mon.mx || s.my !== mon.my || s.scale !== mon.scale || devicePixelRatio !== mon.dpr) {
      const right = hx > sw / 2, fy = hy / sh;
      adoptScreen(s);
      hx = right ? sw - HEAD - EDGE : EDGE; hy = clampY(fy * sh);
      await place(false);
    }
  } catch {}
}
setInterval(() => { if (!level && !drag) { refresh(); watchScreen(); } }, 4000);
document.addEventListener("visibilitychange", () => { if (document.hidden && level) { clearBubbles([...l2, ...l1]); l1 = []; l2 = []; level = 0; stage.classList.remove("open"); place(false); } });

(async function init() {
  const p = savedPos();
  try { adoptScreen(await invoke("head_screen", { mx: p && p.mx != null ? p.mx : null, my: p && p.my != null ? p.my : null })); } catch {}
  hx = p && !p.right ? EDGE : sw - HEAD - EDGE;
  hy = clampY((p ? p.y : 0.42) * sh);
  await refresh();
  await place(false);
  animate(head, { scale: [0.3, 1], opacity: [0, 1] }, spring());
})();
