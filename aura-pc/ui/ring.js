// The "ring my laptop" alert. Dismiss = stop the chime (Rust), which also tells the phone to stop.
const { invoke } = window.__TAURI__.core;
invoke("ring_info").then((who) => {
  if (who) document.getElementById("s").textContent = `${who} is looking for it.`;
}).catch(() => {});
let done = false;
function dismiss() {
  if (done) return;
  done = true;
  invoke("ring_dismiss").catch(() => {});
}
document.addEventListener("click", dismiss);
document.addEventListener("keydown", (e) => {
  if (["Enter", " ", "Escape"].includes(e.key)) { e.preventDefault(); dismiss(); }
});
