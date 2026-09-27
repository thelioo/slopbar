import { t, resetsIn, windowLabel, errorText, creditsText, time, translateDom, setLanguage, setSystemLocale, duration, resetText, groupResets } from "./i18n.js";
import { layoutFor, applyLayout, hitShape, kindOf, mix } from "./shape.js";
import { MARK } from "./marks.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const MAX_H = 580;
const EXPAND_DELAY = 60;
const COLLAPSE_DELAY = 220;

const $island = document.getElementById("island");
const $compact = document.getElementById("compact");
const $compactV = document.getElementById("compact-v");
const $stage = document.getElementById("stage");
const $details = document.getElementById("details");
const $accounts = document.getElementById("accounts");
const $updated = document.getElementById("updated");
const $refresh = document.getElementById("refresh");

// The same page renders the island ("main"), the taskbar widget ("dock") and the tray popover ("panel").
const LABEL = window.__TAURI__.window.getCurrentWindow().label;
const IS_PANEL = LABEL === "panel";
const IS_DOCK = LABEL === "dock";
if (IS_PANEL) document.body.classList.add("panel");
if (IS_DOCK) document.body.classList.add("dock");
const IS_MAIN = !IS_PANEL && !IS_DOCK;
if (IS_MAIN) document.body.classList.add("main");
const pieces = {
  arms: [...document.querySelectorAll(".piece.arm")],
  ears: [...document.querySelectorAll(".piece.ear")],
  island: $island,
};
const DRAG_THRESHOLD = 6;

let expanded = IS_PANEL;
let hoverTimer;
let expandOn = "hover";
let anchor = "top";
let lastSnap = { accounts: [], updated_at: null };

/**
 * SwiftUI-style spring (response, dampingFraction) sampled into a CSS linear() easing.
 * Opening overshoots slightly; closing is critically damped, like the macOS notch apps.
 */
function spring(response, damping, samples = 60) {
  const omega = (2 * Math.PI) / response;
  const duration = Math.min(1.2, 7 / (damping * omega));
  const pts = [];
  for (let i = 0; i <= samples; i++) {
    const t = (i / samples) * duration;
    let x;
    if (damping >= 1) {
      x = 1 - (1 + omega * t) * Math.exp(-omega * t);
    } else {
      const wd = omega * Math.sqrt(1 - damping * damping);
      x = 1 - Math.exp(-damping * omega * t) * (Math.cos(wd * t) + ((damping * omega) / wd) * Math.sin(wd * t));
    }
    pts.push(i === samples ? "1" : x.toFixed(4));
  }
  return { easing: `linear(${pts.join(", ")})`, duration: `${duration.toFixed(3)}s` };
}

if (CSS.supports("transition-timing-function", "linear(0, 1)")) {
  const root = document.documentElement.style;
  for (const [name, s] of Object.entries({
    open: spring(0.42, 0.78),
    close: spring(0.36, 1),
    content: spring(0.5, 0.85),
  })) {
    root.setProperty(`--${name}`, s.easing);
    root.setProperty(`--${name}-dur`, s.duration);
  }
}

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
const color = (p) => (p >= 90 ? "var(--bad)" : p >= 70 ? "var(--warn)" : "var(--ok)");

function ring(p) {
  const r = 6;
  const c = 2 * Math.PI * r;
  const len = (Math.min(100, p) / 100) * c;
  return `<svg class="ring" viewBox="0 0 15 15"><circle class="bg" cx="7.5" cy="7.5" r="${r}"/>
    <circle class="fg" cx="7.5" cy="7.5" r="${r}" stroke="${color(p)}" stroke-dasharray="${len} ${c}"/></svg>`;
}

/** The window closest to its limit is what matters at a glance. */
const peak = (a) => a.windows.reduce((m, w) => (w.used_percent > (m?.used_percent ?? -1) ? w : m), null);

let compactStyle = "names";

const chip = (a) => {
  const w = peak(a);
  const pct = w ? `${Math.round(w.used_percent)}%` : `<span style="color:var(--warn)">!</span>`;
  // Minimal style: only the usage ring, with the provider mark inside it; the name and the
  // number are in the tooltip.
  if (compactStyle === "minimal" || compactStyle === "icons") {
    const tip = w ? `${PROVIDER_NAME[a.provider]} ${Math.round(w.used_percent)}%` : PROVIDER_NAME[a.provider];
    return `<span class="chip" title="${tip}"><span class="ring-mark">${ring(w?.used_percent ?? 0)}${MARK[a.provider]}</span></span>`;
  }
  return `<span class="chip" title="${PROVIDER_NAME[a.provider]}"><span class="name">${PROVIDER_NAME[a.provider]}</span>${w ? ring(w.used_percent) : ""}${pct}</span>`;
};

let toast = null;
let toastTimer;

/** Briefly replaces the collapsed content with a notice (e.g. an automatic account switch). */
function showToast(text) {
  toast = text;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { toast = null; render(lastSnap); }, 4000);
  render(lastSnap);
}

listen("update-installed", ({ payload: version }) => showToast(t("updatedTo", { v: version })));

listen("account-switched", ({ payload: s }) => {
  const name = PROVIDER_NAME[s.provider];
  showToast(t(s.auto ? "switchedAuto" : "switched", { provider: name, to: s.to ?? "" }));
});

/** Chips run across the top bar or down a side bar. */
function renderCompact(snap) {
  let items;
  if (!snap.updated_at) items = [`<span class="dim">${t("loading")}</span>`];
  else if (!snap.accounts.length) items = [`<span class="dim">${t("noAccounts")}</span>`];
  else if (toast) items = [`<span class="chip toast">${esc(toast)}</span>`];
  else {
    const shown = ["claude", "codex"]
      .map((p) => snap.accounts.find((a) => a.provider === p && a.active) ?? snap.accounts.find((a) => a.provider === p))
      .filter(Boolean);
    items = shown.map(chip);
  }
  const kind = IS_MAIN ? kindOf(anchor) : "top";
  const split = kind === "side" ? 0 : items.length;
  $compact.innerHTML = items.slice(0, split).join("");
  $compactV.innerHTML = items.slice(split).join("");
}

const PROVIDER_NAME = { claude: "Claude", codex: "Codex" };

const accountName = (a) => a.alias || a.email || t("unknownAccount");

/** Usage-limit resets the account holds, one line each; green when usable now. */
function resetLines(a) {
  return groupResets(a.resets).map((r) =>
    `<div class="reset-grant ${r.usable ? "usable" : ""}"><span>↺</span>${esc(resetText(r))}</div>`).join("");
}

function bars(a, delay) {
  return a.windows.map((w) => `
    <div class="win">
      <div class="row"><span>${esc(windowLabel(w))}</span><b>${Math.round(w.used_percent)}%</b></div>
      <div class="bar"><div style="width:${Math.min(100, w.used_percent)}%;background:${color(w.used_percent)};transition-delay:${delay}s"></div></div>
      ${w.resets_at ? `<div class="reset">${esc(resetsIn(w.resets_at))}</div>` : ""}
    </div>`).join("");
}

/** One section per provider; each account shows its usage, the signed-in one marked "in use". */
function renderDetails(snap) {
  $updated.textContent = snap.updated_at ? time(snap.updated_at) : "";
  if (!snap.accounts.length) {
    $accounts.innerHTML = `<p class="empty">${snap.updated_at ? t("noCredentials") : t("searching")}</p>`;
    return;
  }
  const groups = ["claude", "codex"]
    .map((p) => [p, snap.accounts.filter((a) => a.provider === p)])
    .filter(([, list]) => list.length);
  let i = 0;
  patch($accounts, groups.map(([provider, list]) => `
    <section style="transition-delay:${0.12 + i++ * 0.05}s">
      <div class="acc-head"><span class="name">${PROVIDER_NAME[provider]}</span></div>
      ${list.map((a) => {
        const stale = a.stale && a.fetched_at
          ? t("staleAgo", { t: duration(Math.max(1, Math.round((Date.now() - new Date(a.fetched_at)) / 60000))) })
          : "";
        // Every row has one detail line, whatever its state, so switching doesn't change heights.
        const meta = [a.active ? a.sources.join(" + ") : a.alias ? a.email : "", a.org, stale].filter(Boolean).join(" · ") || "\u00a0";
        return `
        <div class="acct ${a.active ? "active" : ""}" data-key="${provider}:${esc(a.id)}">
          <div class="acct-head">
            <span class="who">${esc(accountName(a))}</span>
            ${a.plan ? `<span class="plan">${esc(a.plan)}</span>` : ""}
            ${a.active
              ? `<span class="in-use ${a.pending ? "pending" : ""}">${t("inUse")}</span>`
              : `<button class="use" data-provider="${provider}" data-id="${esc(a.id)}">${t("use")}</button>`}
          </div>
          <div class="acc-meta">${esc(meta)}</div>
          ${a.error ? `<div class="error">${esc(errorText(PROVIDER_NAME[provider], a.error))}</div>` : ""}
          ${bars(a, 0.15 + i * 0.05)}
          ${resetLines(a)}
          ${a.credits ? `<div class="reset" style="margin-top:6px">${esc(creditsText(a.credits))}</div>` : ""}
        </div>`;
      }).join("")}
    </section>`).join(""));
}

/**
 * Updates the account list in place: when the same accounts are shown, only rows whose markup
 * changed are replaced, so the others (and their buttons) don't flicker on every refresh.
 */
function patch(container, html) {
  const next = document.createElement("div");
  next.innerHTML = html;
  const keys = (root) => [...root.querySelectorAll("[data-key]")].map((el) => el.dataset.key).join("|");
  const sameRows = container.querySelector("[data-key]") && keys(container) === keys(next);
  if (!sameRows) {
    if (container.innerHTML !== next.innerHTML) container.replaceChildren(...next.childNodes);
    return;
  }
  const fresh = new Map([...next.querySelectorAll("[data-key]")].map((el) => [el.dataset.key, el]));
  for (const row of container.querySelectorAll("[data-key]")) {
    const replacement = fresh.get(row.dataset.key);
    if (replacement && replacement.outerHTML !== row.outerHTML) row.replaceWith(replacement);
  }
}

$accounts.addEventListener("click", async (e) => {
  const button = e.target.closest("button.use");
  if (!button) return;
  const { provider, id } = button.dataset;
  button.disabled = true;
  button.classList.add("busy");
  // Show the new active account right away; the backend confirms (or reverts) it.
  const before = lastSnap;
  holdSize = true;
  render({
    ...lastSnap,
    accounts: lastSnap.accounts.map((a) => (a.provider === provider ? { ...a, active: a.id === id, pending: a.id === id } : a)),
  });
  holdSize = true;
  try {
    const result = await invoke("switch_account", { provider, id });
    holdSize = false;
    render(result);
  } catch (err) {
    holdSize = false;
    render(before);
    showToast(errorText("", String(err)));
  }
});

const compactWidth = () => Math.max(96, Math.ceil($compact.scrollWidth));
const expandedHeight = () => Math.min(MAX_H, Math.ceil($details.scrollHeight));

let lastShape = "";

/** Tells the backend which pixels are the island (sent once per change, never per frame). */
function reportShape(layout) {
  const shape = hitShape(layout);
  const key = JSON.stringify(shape);
  if (key !== lastShape) {
    lastShape = key;
    invoke("set_hit_shape", { shape });
  }
}

/**
 * One physical spring drives the whole island: shape, content and shadow all follow the same
 * progress `p` (0 collapsed, 1 expanded). It keeps its velocity when interrupted, so reversing
 * mid-flight feels continuous instead of restarting. Parameters are SwiftUI's
 * (response, dampingFraction): opening overshoots a touch, closing is critically damped.
 */
const SPRINGS = { open: { response: 0.42, damping: 0.8 }, close: { response: 0.34, damping: 1 } };
const motion = { p: 0, v: 0, target: 0, raf: 0, last: 0 };
let geo = null; // { collapsed, expanded, from, rest, sizes }
/** Size changes while open (content grew or shrank) glide instead of snapping. */
const morph = { t: 1, start: 0, raf: 0 };
const MORPH_MS = 420;
const easeOut = (x) => 1 - (1 - x) ** 4;

function runMorph() {
  morph.start = performance.now();
  morph.t = 0;
  if (morph.raf) return;
  const tick = (now) => {
    morph.t = Math.min(1, (now - morph.start) / MORPH_MS);
    paint();
    morph.raf = morph.t < 1 ? requestAnimationFrame(tick) : 0;
  };
  morph.raf = requestAnimationFrame(tick);
}

/** The open shape as currently drawn (mid-morph if a size change is gliding). */
const openShape = () => (geo.from && morph.t < 1 ? mix(geo.from, geo.expanded, easeOut(morph.t)) : geo.expanded);

const clamp01 = (x) => Math.min(1, Math.max(0, x));

function step(now) {
  const dt = Math.min(0.034, (now - motion.last) / 1000 || 1 / 60);
  motion.last = now;
  const { response, damping } = motion.target ? SPRINGS.open : SPRINGS.close;
  const k = (2 * Math.PI / response) ** 2;
  const c = (4 * Math.PI * damping) / response;
  for (let i = 0; i < 8; i++) {
    const h = dt / 8;
    motion.v += (-k * (motion.p - motion.target) - c * motion.v) * h;
    motion.p += motion.v * h;
  }
  const settled = Math.abs(motion.v) < 0.002 && Math.abs(motion.p - motion.target) < 0.0005;
  if (settled) {
    motion.p = motion.target;
    motion.v = 0;
  }
  paint();
  motion.raf = settled ? 0 : requestAnimationFrame(step);
}

function animateTo(target) {
  motion.target = target;
  if (!motion.raf) {
    motion.last = performance.now();
    motion.raf = requestAnimationFrame(step);
  }
}

/** Draws the frame for the current spring position. */
function paint() {
  if (!geo) return;
  const p = motion.p;
  const q = clamp01(p);
  const shape = mix(geo.collapsed, openShape(), p);
  const box = applyLayout(shape, pieces);

  // Collapsed content stays where it rests on the arms and dissolves quickly.
  const { rest, sizes } = geo;
  if (rest.h) {
    // Whole pixels: half-pixel offsets blur and shift small glyphs.
    $compact.style.left = `${Math.round(rest.h.x - box.x + (rest.h.w - sizes.gw) / 2)}px`;
    $compact.style.top = `${Math.round(rest.h.y - box.y)}px`;
  }
  if (rest.v) {
    $compactV.style.left = `${Math.round(rest.v.x - box.x)}px`;
    $compactV.style.top = `${Math.round(rest.v.y - box.y + (rest.v.h - sizes.gl) / 2)}px`;
  }
  const out = clamp01(1 - q * 2.5);
  for (const el of [$compact, $compactV]) {
    el.style.opacity = out;
    el.style.transform = `scale(${1 + 0.2 * q})`;
    el.style.filter = out > 0 && out < 1 ? `blur(${(1 - out) * 6}px)` : "";
  }

  // The card resolves in as the island opens: fade, a little scale, a little blur.
  const inn = clamp01((q - 0.3) / 0.5);
  $details.style.opacity = inn;
  $details.style.transform = `translate(var(--tx), var(--ty)) scale(${0.92 + 0.08 * q})`;
  $details.style.filter = inn > 0 && inn < 1 ? `blur(${(1 - inn) * 6}px)` : "";

  // Each arm casts its own shadow, so it follows the exact shape as it morphs. Where one arm's
  // shadow falls on another it lands on black and disappears, reading as a single shadow.
  const shadow = q > 0.01 ? `0 ${12 * q}px ${40 * q}px rgba(0,0,0,${0.4 * q})` : "none";
  for (const arm of pieces.arms) arm.style.boxShadow = shadow;
}

/** While an account switch is in flight, the shape keeps its size and resizes once at the end. */
let holdSize = false;

/** Recomputes both end states (content may have changed) and repaints at the current position. */
function layout() {
  if (holdSize && geo) return;
  if (IS_DOCK) {
    invoke("set_dock_width", { width: Math.ceil($compact.scrollWidth) });
    return;
  }
  if (IS_PANEL) {
    $island.classList.add("expanded");
    invoke("set_panel_height", { height: Math.ceil($details.scrollHeight) });
    return;
  }
  const W = window.innerWidth, H = window.innerHeight;
  const sizes = {
    gw: $compact.childElementCount ? Math.ceil($compact.scrollWidth) : 0,
    gl: $compactV.childElementCount ? Math.ceil($compactV.scrollHeight) : 0,
    dh: expandedHeight(),
  };
  const collapsed = layoutFor(anchor, { ...sizes, expanded: false }, W, H);
  const expandedL = layoutFor(anchor, { ...sizes, expanded: true }, W, H);
  const previous = geo;
  const sameShape = previous && previous.anchor === anchor;
  const from = sameShape && motion.p > 0.01 ? openShape() : null;
  geo = { collapsed, expanded: expandedL, from, rest: collapsed, sizes, anchor };
  if (from && JSON.stringify(from.arms) !== JSON.stringify(expandedL.arms)) runMorph();

  $stage.style.transformOrigin = `${collapsed.origin[0]}px ${collapsed.origin[1]}px`;

  // The card grows out of the attached edge.
  const [hx, vy] = expandedL.card;
  Object.assign($details.style, {
    left: hx === "center" ? "50%" : hx === "left" ? "0" : "auto",
    right: hx === "right" ? "0" : "auto",
    top: vy === "center" ? "50%" : vy === "top" ? "0" : "auto",
    bottom: vy === "bottom" ? "0" : "auto",
  });
  $details.style.setProperty("--tx", hx === "center" ? "-50%" : "0px");
  $details.style.setProperty("--ty", vy === "center" ? "-50%" : "0px");
  $details.style.setProperty("--origin", `${vy} ${hx}`);

  reportShape(expanded ? expandedL : collapsed);
  paint();
}

function setExpanded(value) {
  if (expanded === value) return;
  expanded = value;
  $island.classList.toggle("expanded", value);
  if (IS_MAIN) document.body.classList.toggle("expanded", value);
  if (!value) $details.scrollTop = 0;
  layout();
  if (IS_MAIN) animateTo(value ? 1 : 0);
}

function render(snap) {
  lastSnap = snap;
  renderCompact(snap);
  renderDetails(snap);
  layout();
}

listen("island-hover", ({ payload: inside }) => {
  if (IS_PANEL || IS_DOCK) return;
  clearTimeout(hoverTimer);
  if (inside && expandOn === "click") return;
  hoverTimer = setTimeout(() => setExpanded(inside), inside ? EXPAND_DELAY : COLLAPSE_DELAY);
});

/**
 * Press and move turns the island (or the taskbar widget) into a pill that follows the cursor;
 * the backend decides where it lands on release. A press without movement is a click.
 */
let press = null;
let dragged = false;
$island.addEventListener("pointerdown", (e) => {
  if (IS_PANEL || e.button !== 0 || e.target.closest("button")) return;
  press = { x: e.screenX, y: e.screenY };
  dragged = false;
});
window.addEventListener("pointermove", (e) => {
  if (!press || dragged) return;
  if (Math.hypot(e.screenX - press.x, e.screenY - press.y) < DRAG_THRESHOLD) return;
  dragged = true;
  press = null;
  document.body.classList.add("dragging");
  clearTimeout(hoverTimer);
  if (!IS_DOCK) setExpanded(false);
  // The dragged pill lays every chip out in a row.
  const width = IS_DOCK ? compactWidth() + 32 : Math.max(160, $compact.scrollWidth + $compactV.scrollHeight);
  invoke("start_drag", { width });
});
window.addEventListener("pointerup", () => {
  if (!press) return;
  press = null;
  if (IS_DOCK) invoke("dock_clicked");
  else if (expandOn === "click") setExpanded(!expanded);
});

/** Landing: the pill arrives slightly large and blurred, then springs into focus. */
function landing() {
  (IS_MAIN ? $stage : $island).animate(
    [
      { transform: "scale(1.18)", opacity: 0 },
      { transform: "scale(0.97)", opacity: 1, offset: 0.6 },
      { transform: "scale(1)", opacity: 1 },
    ],
    { duration: 560, easing: "cubic-bezier(0.2, 0, 0, 1)" },
  );
}
listen("drop-enter", () => {
  document.body.classList.remove("dragging");
  if (IS_PANEL) return;
  if (IS_DOCK ? !document.body.classList.contains("floating") : true) landing();
});

/** While dragged, the pill leans and stretches with the cursor's velocity, with motion blur. */
let target = { x: 0, y: 0 };
let current = { x: 0, y: 0 };
let flying = false;
function flight() {
  if (!flying) return;
  current.x += (target.x - current.x) * 0.25;
  current.y += (target.y - current.y) * 0.25;
  target.x *= 0.85; // velocity decays when the cursor stops
  target.y *= 0.85;
  const speed = Math.hypot(current.x, current.y);
  const tilt = Math.max(-10, Math.min(10, current.x / 120));
  const stretch = Math.min(0.18, speed / 9000);
  const blur = Math.min(3, speed / 1500);
  const sx = -current.x / 150, sy = 18 - current.y / 150;
  $island.style.transform = `scale(1.06) rotate(${tilt}deg) scale(${1 + stretch}, ${1 - stretch / 2})`;
  $island.style.filter = blur > 0.2 ? `blur(${blur.toFixed(2)}px)` : "";
  $island.style.boxShadow = `${sx.toFixed(1)}px ${sy.toFixed(1)}px 36px rgba(0,0,0,0.45)`;
  requestAnimationFrame(flight);
}

if (IS_DOCK) {
  listen("dock-velocity", ({ payload: [vx, vy] }) => (target = { x: vx, y: vy }));
  listen("dock-floating", ({ payload }) => {
    document.body.classList.toggle("floating", payload);
    flying = payload;
    target = { x: 0, y: 0 };
    current = { x: 0, y: 0 };
    if (payload) {
      // Lift: the pill rises off the screen before it follows the cursor.
      $island.animate(
        [
          { transform: "scale(1)", boxShadow: "0 0 0 rgba(0,0,0,0)" },
          { transform: "scale(1.06)", boxShadow: "0 18px 36px rgba(0,0,0,0.45)" },
        ],
        { duration: 260, easing: "cubic-bezier(0.2, 0, 0, 1)" },
      ).finished.then(() => requestAnimationFrame(flight));
    } else {
      $island.style.transform = $island.style.filter = $island.style.boxShadow = "";
      landing();
    }
  });
  listen("dock-zone", ({ payload }) => (document.body.dataset.zone = payload));
  const theme = () => invoke("taskbar_theme").then((t) => (document.body.dataset.taskbar = t));
  theme();
  setInterval(theme, 10_000);
}

document.getElementById("open-settings").addEventListener("click", () => invoke("open_settings"));

function applySettings(settings) {
  expandOn = settings.expand_on;
  compactStyle = settings.compact_style ?? "names";
  if (IS_MAIN) {
    anchor = settings.anchor;
    document.body.dataset.anchor = anchor;
  }
  setLanguage(settings.language);
  translateDom();
  if (!IS_PANEL) {
    const labels = Object.fromEntries(["show", "refresh", "settings", "quit"].map((k) => [k, t(k)]));
    invoke("set_tray_labels", { labels });
  }
  render(lastSnap);
}

listen("settings-changed", (e) => applySettings(e.payload));
invoke("system_locale")
  .then(setSystemLocale, () => {})
  .finally(() => invoke("get_settings").then(applySettings));

$refresh.addEventListener("click", async () => {
  $refresh.classList.add("spin");
  try { render(await invoke("refresh")); } finally { $refresh.classList.remove("spin"); }
});


listen("usage-updated", (e) => render(e.payload));
invoke("get_usage").then(render);
setInterval(() => invoke("get_usage").then(render), 60_000);

// Replay the pop-in each time the tray panel opens.
if (IS_PANEL) {
  window.addEventListener("focus", () => {
    $details.style.animation = "none";
    void $details.offsetWidth;
    $details.style.animation = "";
  });
}
