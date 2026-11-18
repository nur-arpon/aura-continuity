// "More from your phone" bubble: spring in, drag-to-dismiss, auto-hide with pause on hover.
const { invoke } = window.__TAURI__.core;
const { animate } = window.Motion;
let d = {};
try { d = JSON.parse(decodeURIComponent(location.hash.slice(1))); } catch (_) {}

const $ = (id) => document.getElementById(id);
const card = $("card");
const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
$("t1").textContent = ["From your phone", d.sub, d.pos].filter(Boolean).join(" · ");
$("t2").textContent = d.channel ? `${d.title} – ${d.channel}` : (d.title || "Something new");
if (d.more > 0) $("go").textContent = "Open";

const SPRING = { type: "spring", stiffness: 380, damping: 32, mass: 0.9 };
let closing = false;

// in: slide up from the corner with a little scale, interruptible
if (reduce) animate(card, { opacity: [0, 1] }, { duration: 0.12 });
else animate(card, { opacity: [0, 1], x: [48, 0], y: [20, 0], scale: [0.94, 1] }, { ...SPRING, opacity: { duration: 0.18 } });

// auto-hide: the progress bar runs down; hovering pauses it
const bar = animate($("bar"), { scaleX: [1, 0] }, { duration: (d.ms || 8000) / 1000, ease: "linear" });
bar.then(() => dismiss(0));
card.addEventListener("pointerenter", () => bar.pause());
card.addEventListener("pointerleave", () => { if (!dragging) bar.play(); });

async function dismiss(dir) {
  if (closing) return;
  closing = true;
  bar.stop();
  if (reduce) await animate(card, { opacity: 0 }, { duration: 0.1 });
  else await animate(card, { opacity: 0, x: dir >= 0 ? 90 : -90, scale: 0.96 }, { type: "spring", stiffness: 420, damping: 38 });
  invoke("zt_bubble_close").catch(() => {});
}

// drag right (or left) to dismiss; a release short of the threshold springs back with the throw velocity
let dragging = false, sx = 0, dx = 0, lastX = 0, lastT = 0, vx = 0, moved = false;
card.addEventListener("pointerdown", (e) => {
  if (e.target.id === "x") return;
  dragging = true; moved = false; sx = e.clientX; dx = 0; lastX = e.clientX; lastT = e.timeStamp; vx = 0;
  card.setPointerCapture(e.pointerId); bar.pause();
});
card.addEventListener("pointermove", (e) => {
  if (!dragging) return;
  dx = e.clientX - sx;
  if (Math.abs(dx) > 4) moved = true;
  const dt = Math.max(1, e.timeStamp - lastT); vx = (e.clientX - lastX) / dt * 1000; lastX = e.clientX; lastT = e.timeStamp;
  // rubber band against the pointer the further it goes
  animate(card, { x: dx * 0.85, opacity: 1 - Math.min(0.6, Math.abs(dx) / 400) }, { duration: 0 });
});
card.addEventListener("pointerup", (e) => {
  if (!dragging) return;
  dragging = false; card.releasePointerCapture(e.pointerId);
  if (Math.abs(dx) > 90 || Math.abs(vx) > 700) return dismiss(dx || vx);
  animate(card, { x: 0, opacity: 1 }, { ...SPRING, velocity: vx });
  if (!moved) invoke("zt_recent_open", { index: d.index || 0 }).catch(() => {});
  else bar.play();
});
$("x").addEventListener("click", (e) => { e.stopPropagation(); dismiss(1); });
addEventListener("keydown", (e) => { if (e.key === "Escape") dismiss(1); });
