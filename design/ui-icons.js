// Continuity UI icon set - one inline SVG sprite, 20 px grid, 1.5 px stroke,
// round caps (Fluent System Icons proportions, drawn for this family).
// Source of truth: design\ui-icons.js (synced into each ui\ by sync.ps1).
// Use: <svg class="ic" aria-hidden="true"><use href="#i-lock"/></svg>
(() => {
  const P = {
    lock: '<rect x="4.75" y="8.75" width="10.5" height="8" rx="2"/><path d="M7 8.75V6.5a3 3 0 0 1 6 0v2.25"/><path d="M10 12v1.5"/>',
    "lock-test": '<rect x="4.75" y="8.75" width="10.5" height="8" rx="2" stroke-dasharray="2.2 1.8"/><path d="M7 8.75V6.5a3 3 0 0 1 6 0v2.25"/>',
    shield: '<path d="M10 2.75 4.25 5v4.5c0 3.6 2.4 6.3 5.75 7.75 3.35-1.45 5.75-4.15 5.75-7.75V5L10 2.75Z"/><path d="m7.5 10 1.75 1.75L12.75 8.5"/>',
    timer: '<circle cx="10" cy="11" r="6.25"/><path d="M10 7.75V11l2 1.5M8 2.75h4"/>',
    pause: '<circle cx="10" cy="10" r="7.25"/><path d="M8.25 7.5v5M11.75 7.5v5"/>',
    "bt-off": '<path d="m6.5 6.5 7 6.5-3.5 3.25V10M10 7.5V3.75L13.5 7l-1.6 1.5M6.5 13.5 9 11.2M3.5 3.5l13 13"/>',
    bluetooth: '<path d="m6.5 6.5 7 6.5-3.5 3.25V3.75L13.5 7l-7 6.5"/>',
    phone: '<rect x="5.75" y="2.75" width="8.5" height="14.5" rx="2"/><path d="M9 14.75h2"/>',
    watch: '<rect x="5.75" y="6" width="8.5" height="8" rx="2.25"/><path d="M7.5 6l.5-3.25h4L12.5 6M7.5 14l.5 3.25h4l.5-3.25"/>',
    headphones: '<path d="M3.75 13v-2.5a6.25 6.25 0 0 1 12.5 0V13"/><rect x="3.75" y="11.5" width="3.5" height="5" rx="1.25"/><rect x="12.75" y="11.5" width="3.5" height="5" rx="1.25"/>',
    laptop: '<rect x="4.25" y="4.25" width="11.5" height="8" rx="1.5"/><path d="M2.75 15.75h14.5"/>',
    keyboard: '<rect x="2.75" y="5.25" width="14.5" height="9.5" rx="2"/><path d="M6 8.5h.01M8.67 8.5h.01M11.33 8.5h.01M14 8.5h.01M7 11.75h6"/>',
    folder: '<path d="M2.75 6.25a1.5 1.5 0 0 1 1.5-1.5h3.1l1.9 1.9h6.5a1.5 1.5 0 0 1 1.5 1.5v6.6a1.5 1.5 0 0 1-1.5 1.5H4.25a1.5 1.5 0 0 1-1.5-1.5Z"/>',
    refresh: '<path d="M15.75 10a5.75 5.75 0 1 1-1.7-4.07M15.75 3.75v3.5h-3.5"/>',
    warning: '<path d="M8.7 3.6a1.5 1.5 0 0 1 2.6 0l5.9 10.6a1.5 1.5 0 0 1-1.3 2.25H4.1a1.5 1.5 0 0 1-1.3-2.25Z"/><path d="M10 8v3M10 13.75h.01"/>',
    check: '<circle cx="10" cy="10" r="7.25"/><path d="m7 10.25 2 2 4-4.25"/>',
    info: '<circle cx="10" cy="10" r="7.25"/><path d="M10 9v4.25M10 6.75h.01"/>',
    power: '<path d="M10 2.75V9M6.1 5.2a6.25 6.25 0 1 0 7.8 0"/>',
    globe: '<circle cx="10" cy="10" r="7.25"/><path d="M2.75 10h14.5M10 2.75c-2.1 2.2-2.9 4.6-2.9 7.25s.8 5.05 2.9 7.25c2.1-2.2 2.9-4.6 2.9-7.25S12.1 4.95 10 2.75Z"/>',
    play: '<rect x="2.75" y="4.25" width="14.5" height="11.5" rx="2.5"/><path d="m8.5 7.75 4 2.25-4 2.25Z"/>',
    bell: '<path d="M10 2.75a4.5 4.5 0 0 0-4.5 4.5v2.5L4 12.5v1h12v-1l-1.5-2.75v-2.5A4.5 4.5 0 0 0 10 2.75Z"/><path d="M8.25 15.75a1.9 1.9 0 0 0 3.5 0"/>',
    volume: '<path d="M3.75 8.25v3.5h2.75L10 15V5L6.5 8.25H3.75Z"/><path d="M12.75 7.5a3.5 3.5 0 0 1 0 5M14.75 5.5a6.25 6.25 0 0 1 0 9"/>',
    slides: '<rect x="3" y="4" width="14" height="9.5" rx="1.75"/><path d="M10 13.5v3M7 16.75h6M7.5 9h5"/>',
    playtri: '<path d="M6.5 4.5l9 5.5-9 5.5V4.5Z"/>',
    next: '<path d="M5 5l6 5-6 5V5ZM14.5 5v10"/>',
    prev: '<path d="M15 5l-6 5 6 5V5ZM5.5 5v10"/>',
    send: '<path d="M3.5 10h6.25M3.5 10 2.9 4.1a.6.6 0 0 1 .85-.6l12.6 6a.6.6 0 0 1 0 1.08l-12.6 6a.6.6 0 0 1-.85-.6Z"/>',
    handoff: '<rect x="11.25" y="6.75" width="6" height="10.5" rx="1.5"/><path d="M2.75 9.5a6 6 0 0 1 6-6h1.5M8.5 1.75l1.75 1.75L8.5 5.25"/>',
    inbox: '<path d="M2.75 11.25 4.6 4.6a1.5 1.5 0 0 1 1.45-1.1h7.9a1.5 1.5 0 0 1 1.45 1.1l1.85 6.65v3.5a1.5 1.5 0 0 1-1.5 1.5H4.25a1.5 1.5 0 0 1-1.5-1.5Z"/><path d="M2.75 11.25h4l1 2h4.5l1-2h4"/>',
    clock: '<circle cx="10" cy="10" r="7.25"/><path d="M10 6.25V10l2.5 1.75"/>',
    chevron: '<path d="m7.75 5.5 4.5 4.5-4.5 4.5"/>',
    unlock: '<rect x="4.75" y="8.75" width="10.5" height="8" rx="2"/><path d="M7 8.75V6.5a3 3 0 0 1 5.6-1.5"/><path d="M10 12v1.5"/>',
    close: '<path d="m5.5 5.5 9 9M14.5 5.5l-9 9"/>',
    copy: '<rect x="7.25" y="7.25" width="9" height="9" rx="2"/><path d="M12.75 7.25V5.5a2 2 0 0 0-2-2H5.5a2 2 0 0 0-2 2v5.25a2 2 0 0 0 2 2h1.75"/>',
    trash: '<path d="M4.5 6h11M8 6V4.25h4V6M5.75 6l.6 9.1a1.5 1.5 0 0 0 1.5 1.4h4.3a1.5 1.5 0 0 0 1.5-1.4l.6-9.1"/>',
    link: '<path d="M8.5 11.5a3 3 0 0 0 4.2 0l2.3-2.3a3 3 0 0 0-4.2-4.2l-.7.7M11.5 8.5a3 3 0 0 0-4.2 0L5 10.8a3 3 0 0 0 4.2 4.2l.7-.7"/>',
  };
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("aria-hidden", "true");
  svg.style.cssText = "position:absolute;width:0;height:0;overflow:hidden";
  svg.innerHTML = Object.entries(P)
    .map(([k, v]) => `<symbol id="i-${k}" viewBox="0 0 20 20">${v}</symbol>`)
    .join("");
  document.body.prepend(svg);
})();
