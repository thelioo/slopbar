import { t, resetsIn, windowLabel, errorText, creditsText, time, translateDom, setLanguage, setSystemLocale, duration, resetText, groupResets } from "./i18n.js";
import { MARK } from "./marks.js";
import { checkUsage, checkSwitch } from "./alerts.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $widget = document.getElementById("widget");
const $compact = document.getElementById("compact");
const $details = document.getElementById("details");
const $accounts = document.getElementById("accounts");
const $updated = document.getElementById("updated");
const $refresh = document.getElementById("refresh");

// The same page renders the taskbar widget ("dock") and the flyout above it ("panel").
const IS_DOCK = window.__TAURI__.window.getCurrentWindow().label === "dock";
document.body.classList.add(IS_DOCK ? "dock" : "panel");

let lastSnap = { accounts: [], updated_at: null };
/** Which notifications to show (from settings); only the widget sends them. */
let notifications = null;

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

/** The usage ring with the provider's mark inside; the name and the number are in the tooltip. */
const chip = (a) => {
  const w = peak(a);
  const tip = w ? `${PROVIDER_NAME[a.provider]} ${Math.round(w.used_percent)}%` : PROVIDER_NAME[a.provider];
  return `<span class="chip" title="${tip}"><span class="ring-mark">${ring(w?.used_percent ?? 0)}${MARK[a.provider]}</span></span>`;
};

let toast = null;
let toastTimer;

/** Briefly replaces the widget's content with a notice (e.g. an automatic account switch). */
function showToast(text) {
  toast = text;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { toast = null; render(lastSnap); }, 4000);
  render(lastSnap);
}

listen("update-installed", ({ payload: version }) => showToast(t("updatedTo", { v: version })));

listen("account-switched", ({ payload: s }) => {
  if (IS_DOCK && notifications) checkSwitch(s, notifications);
  const name = PROVIDER_NAME[s.provider];
  showToast(t(s.auto ? "switchedAuto" : "switched", { provider: name, to: s.to ?? "" }));
});

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
  $compact.innerHTML = items.join("");
}

const PROVIDER_NAME = { claude: "Claude", codex: "Codex" };

const accountName = (a) => a.alias || a.email || t("unknownAccount");

/** Usage-limit resets the account holds, one line each; green when usable now. */
function resetLines(a) {
  return groupResets(a.resets).map((r) =>
    `<div class="reset-grant ${r.usable ? "usable" : ""}"><span>↺</span>${esc(resetText(r))}</div>`).join("");
}

function bars(a) {
  return a.windows.map((w) => `
    <div class="win">
      <div class="row"><span>${esc(windowLabel(w))}</span><b>${Math.round(w.used_percent)}%</b></div>
      <div class="bar"><div style="width:${Math.min(100, w.used_percent)}%;background:${color(w.used_percent)}"></div></div>
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
  patch($accounts, groups.map(([provider, list]) => `
    <section>
      <div class="acc-head"><span class="name">${PROVIDER_NAME[provider]}</span></div>
      ${list.map((a) => {
        const stale = a.stale && a.fetched_at
          ? t("staleAgo", { t: duration(Math.max(1, Math.round((Date.now() - new Date(a.fetched_at)) / 60000))) })
          : "";
        // Every row has one detail line, whatever its state, so switching doesn't change heights.
        const meta = [a.active ? a.sources.join(" + ") : a.alias ? a.email : "", a.org, stale].filter(Boolean).join(" · ") || " ";
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
          ${bars(a)}
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
  render({
    ...lastSnap,
    accounts: lastSnap.accounts.map((a) => (a.provider === provider ? { ...a, active: a.id === id, pending: a.id === id } : a)),
  });
  try {
    render(await invoke("switch_account", { provider, id }));
  } catch (err) {
    render(before);
    showToast(errorText("", String(err)));
  }
});

/** Sizes the window to its content: the widget's width, or the flyout's height. */
function layout() {
  if (IS_DOCK) invoke("set_dock_width", { width: Math.ceil($widget.offsetWidth) });
  else invoke("set_panel_height", { height: Math.ceil($details.scrollHeight) });
}

function render(snap) {
  lastSnap = snap;
  if (IS_DOCK && notifications && snap.updated_at) checkUsage(snap, notifications);
  if (IS_DOCK) renderCompact(snap);
  else renderDetails(snap);
  layout();
}

// Left click opens the flyout, right click the context menu, like the built-in tray items.
if (IS_DOCK) {
  $widget.addEventListener("click", () => invoke("dock_clicked"));
  listen("panel-visible", ({ payload }) => $widget.classList.toggle("open", payload));
}
window.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  if (IS_DOCK) invoke("dock_menu");
});

/** Both windows follow the taskbar's theme, like the shell's own flyouts. */
const theme = () => invoke("taskbar_theme").then((t) => (document.documentElement.dataset.theme = t));
theme();
setInterval(theme, 10_000);

document.getElementById("open-settings").addEventListener("click", () => invoke("open_settings"));

function applySettings(settings) {
  notifications = settings.notifications;
  setLanguage(settings.language);
  translateDom();
  if (IS_DOCK) {
    const labels = Object.fromEntries(["refresh", "settings", "quit"].map((k) => [k, t(k)]));
    invoke("set_menu_labels", { labels });
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

// Replay the flyout's entrance each time it opens.
if (!IS_DOCK) {
  window.addEventListener("focus", () => {
    $details.style.animation = "none";
    void $details.offsetWidth;
    $details.style.animation = "";
  });
}
