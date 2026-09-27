// A fake 1280x720 Windows desktop for the scripted demos (index.html, accounts.html): wallpaper,
// a Windows 11 taskbar with the real widget (app.html?label=dock) next to the clock, the real
// flyout (app.html?label=panel) above it, and a cursor the timeline moves around. The pages
// load backend.js first and mark their root data-slopbar-host, so both frames share it.

const W = 1280, H = 720, BAR = 48, WORK_H = H - BAR, MARGIN = 12, FLYOUT_W = 380;
const { state, hooks, emit } = window.slopbar;

document.head.insertAdjacentHTML("beforeend", `<style>
  * { box-sizing: border-box; margin: 0; }
  html, body { width: ${W}px; height: ${H}px; overflow: hidden; }
  body {
    font: 12px/1.35 "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
    background:
      radial-gradient(60% 80% at 18% 28%, #4a57b8 0%, transparent 60%),
      radial-gradient(50% 70% at 88% 64%, #c2507a 0%, transparent 60%),
      radial-gradient(70% 90% at 58% 115%, #f09a48 0%, transparent 55%),
      #1b1c35;
  }
  :root { --tb: rgba(28, 28, 28, 0.85); --tb-ink: #fff; --fly: rgba(44, 44, 44, 0.86); --fly-stroke: rgba(0, 0, 0, 0.2); }
  :root[data-taskbar="light"] { --tb: rgba(238, 238, 238, 0.85); --tb-ink: #1b1b1b; --fly: rgba(249, 249, 249, 0.86); --fly-stroke: rgba(0, 0, 0, 0.0578); }
  iframe { display: block; border: 0; background: transparent; color-scheme: dark; }
  :root[data-taskbar="light"] iframe { color-scheme: light; }
  .taskbar {
    position: absolute; left: 0; right: 0; bottom: 0; height: ${BAR}px; display: flex; justify-content: flex-end;
    background: var(--tb); color: var(--tb-ink); backdrop-filter: blur(30px) saturate(1.4);
    border-top: 1px solid rgba(255, 255, 255, 0.08); transition: background 0.3s, color 0.3s;
  }
  .apps { position: absolute; left: 50%; top: 8px; transform: translateX(-50%); display: flex; gap: 6px; }
  .apps i { width: 32px; height: 32px; border-radius: 6px; display: block; }
  #dock { width: 160px; height: ${BAR}px; }
  .tray { display: flex; align-items: center; gap: 14px; padding: 0 12px 0 8px; font-size: 11.5px; line-height: 1.3; }
  .tray .icons { display: flex; gap: 10px; }
  .tray .icons i { width: 14px; height: 14px; border-radius: 3px; background: currentColor; opacity: 0.6; display: block; }
  .tray .clock { text-align: right; }
  .flyout {
    position: absolute; width: ${FLYOUT_W}px; border-radius: 8px; overflow: hidden; opacity: 0; transition: opacity 0.1s;
    background: var(--fly); border: 1px solid var(--fly-stroke); backdrop-filter: blur(30px) saturate(1.5);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  }
  .flyout.open { opacity: 1; }
  .flyout iframe { width: 100%; height: 100%; }
  .cursor { position: absolute; left: 0; top: 0; width: 22px; height: 22px; pointer-events: none; z-index: 10;
    filter: drop-shadow(0 1px 1.5px rgba(0, 0, 0, 0.45)); transition: scale 0.12s; }
  .cursor.down { scale: 0.85; }
</style>`);

document.body.insertAdjacentHTML("beforeend", `
  <div class="flyout" id="flyout"><iframe id="panel" src="app.html?label=panel" allowtransparency="true"></iframe></div>
  <div class="taskbar">
    <div class="apps">
      <i style="background:#2b7cd3"></i><i style="background:#f2b01e"></i><i style="background:#1e9e5a"></i>
      <i style="background:#7a5af5"></i><i style="background:#e8453c"></i>
    </div>
    <iframe id="dock" src="app.html?label=dock" allowtransparency="true"></iframe>
    <div class="tray">
      <div class="icons"><i></i><i></i><i></i></div>
      <div class="clock">9:41 AM<br />9/27/2026</div>
    </div>
  </div>
  <svg class="cursor" id="cursor" viewBox="0 0 22 22"><path d="M3 2 L3 18 L7.2 14.2 L10 20.5 L12.8 19.3 L10.1 13.2 L15.8 13.2 Z" fill="#fff" stroke="#000" stroke-width="1.2" stroke-linejoin="round"/></svg>`);

const $ = (id) => document.getElementById(id);
const dock = $("dock"), panel = $("panel"), flyout = $("flyout"), cursor = $("cursor");
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);

// ---------------------------------------------------------------------------------------------
// The windows, placed as the app places them

let panelHeight = 320;
function placeFlyout() {
  const center = dock.offsetLeft + dock.offsetWidth / 2;
  const x = Math.min(Math.max(center - FLYOUT_W / 2, MARGIN), W - FLYOUT_W - MARGIN);
  Object.assign(flyout.style, { left: `${x}px`, top: `${WORK_H - panelHeight - MARGIN}px`, height: `${panelHeight}px` });
}
hooks.set_dock_width = ({ width }) => { if (width > 20) dock.style.width = `${width}px`; placeFlyout(); };
hooks.set_panel_height = ({ height }) => { panelHeight = height; placeFlyout(); };
hooks.dock_clicked = () => setFlyout(!flyout.classList.contains("open"));

export function setFlyout(open) {
  flyout.classList.toggle("open", open);
  // The app replays the flyout's entrance when its window gets focus.
  if (open) panel.contentWindow.focus();
  emit("panel-visible", open);
}

/** Switches the taskbar between light and dark; the widget and the flyout follow it. */
export function setTheme(theme) {
  state.theme = theme;
  document.documentElement.dataset.taskbar = theme;
  for (const frame of [dock, panel]) frame.contentDocument.documentElement.dataset.theme = theme;
}

/** Applies a settings change, as saving it from the Settings window would. */
export function setSettings(patch) {
  state.settings = { ...state.settings, ...patch };
  emit("settings-changed", structuredClone(state.settings));
}

// ---------------------------------------------------------------------------------------------
// The cursor

let cx = 760, cy = 330;
const place = () => (cursor.style.transform = `translate(${cx - 3}px, ${cy - 2}px)`);
place();

export function move(x, y, ms) {
  const [x0, y0] = [cx, cy];
  const start = performance.now();
  return new Promise((done) => {
    const tick = (now) => {
      const t = Math.min(1, (now - start) / ms);
      cx = x0 + (x - x0) * ease(t);
      cy = y0 + (y - y0) * ease(t);
      place();
      t < 1 ? requestAnimationFrame(tick) : done();
    };
    requestAnimationFrame(tick);
  });
}

export async function click(el) {
  cursor.classList.add("down");
  await sleep(110);
  el.click();
  cursor.classList.remove("down");
}

/** Center of an element inside one of the frames, in desktop px. */
export function centerOf(frame, el) {
  const f = frame.getBoundingClientRect(), r = el.getBoundingClientRect();
  return [f.left + r.left + r.width / 2, f.top + r.top + r.height / 2];
}

const loaded = (frame) => new Promise((r) => (frame.contentDocument?.readyState === "complete" && frame.contentDocument.location.href !== "about:blank" ? r() : frame.addEventListener("load", r)));
await Promise.all([loaded(dock), loaded(panel)]);
setTheme(state.theme);

export const frames = { dock, panel };
export const widget = () => dock.contentDocument.getElementById("widget");
