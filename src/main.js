import { t, windowResetText, windowLabel, errorText, creditsText, time, dayTime, translateDom, setLanguage, setSystemLocale, duration, resetText, groupResets, visibleResetWindows, placesText } from "./i18n.js";
import { MARK } from "./marks.js";
import { observeFrontend } from "./diagnostics.js";
import { creditAdviceText } from "./alerts.js";
import { sparkline } from "./sparkline.js";
import { accountTip } from "./accounttext.js";

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
observeFrontend(IS_DOCK ? "dock" : "panel");
document.body.classList.add(IS_DOCK ? "dock" : "panel");

let lastSnap = { accounts: [], updated_at: null };
/** The whole settings, for the flyout's layout choice. */
let current = null;
/** Each account's weekly usage over the last 7 days, keyed `provider:id` (get_sparklines). */
let trails = {};
/** Each account's get_subscriptions entry, keyed `provider:id`, for the compact tooltips. */
let subs = {};
/** The compact layout is offered above this many accounts. */
const COMPACT_OFFER_ABOVE = 8;

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
  const w = a.stale || a.error ? null : peak(a);
  const tip = w ? `${PROVIDER_NAME[a.provider]} ${Math.round(w.used_percent)}%` : `${PROVIDER_NAME[a.provider]} — ${t("usageUnknown")}`;
  return `<span class="chip" role="img" aria-label="${esc(tip)}" title="${esc(tip)}"><span class="ring-mark">${w ? ring(w.used_percent) : '<span class="unknown-mark">?</span>'}${MARK[a.provider]}</span></span>`;
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

// The menus' Compact flyout item or its shortcut: the flyout is usually closed, so say which.
if (IS_DOCK) listen("compact-toggled", ({ payload: on }) => showToast(t(on ? "compactOn" : "compactOff")));

listen("account-switched", ({ payload: s }) => {
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
  $widget.setAttribute("aria-label", t("title"));
}

const PROVIDER_NAME = { claude: "Claude", codex: "Codex" };

const accountName = (a) => a.alias || a.email || t("unknownAccount");

/** Usage-limit resets the account holds, one line each; green when usable now. */
let creditAdvice = {};
/** Spend-or-switch advice for Codex accounts at their limit with a reset credit; re-renders when it changes. */
function loadCreditAdvice() {
  invoke("get_credit_advice").then((list) => {
    const next = Object.fromEntries((list ?? []).map((c) => [`${c.provider}:${c.id}`, creditAdviceText(c)]));
    if (JSON.stringify(next) === JSON.stringify(creditAdvice)) return;
    creditAdvice = next;
    renderDetails(lastSnap);
    layout();
  }, () => {});
}

function resetLines(a) {
  return groupResets(a.resets).map((r) =>
    `<div class="reset-grant ${r.usable ? "usable" : ""}"><span>↺</span>${esc(resetText(r))}</div>`).join("");
}

function bars(a) {
  const resetWindows = new Set(visibleResetWindows(a.windows));
  return a.windows.map((w) => `
    <div class="win">
      <div class="row"><span>${esc(windowLabel(w))}</span><b>${Math.round(w.used_percent)}%</b></div>
      <div class="bar" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(Math.min(100, w.used_percent))}"
        aria-label="${esc(`${windowLabel(w)} ${t("used")}`)}"><div style="width:${Math.min(100, w.used_percent)}%;background:${color(w.used_percent)}"></div></div>
      ${resetWindows.has(w) && !a.stale ? `<div class="reset" title="${esc(w.resets_at)}">${esc(windowResetText(w, false, a.fetched_at))}</div>` : ""}
    </div>`).join("");
}

/** One section per provider; each account shows its usage, the signed-in one marked "in use". */
function renderDetails(snap) {
  const notice = document.getElementById("panel-notice");
  notice.textContent = toast ?? "";
  notice.hidden = !toast;
  $updated.textContent = snap.updated_at ? time(snap.updated_at) : "";
  renderCompactOffer(snap);
  if (!snap.accounts.length) {
    $accounts.innerHTML = `<p class="empty">${snap.updated_at ? t("noCredentials") : t("searching")}</p>`;
    return;
  }
  // Rows are patched in place only within one layout (see patch).
  const layout = current?.compact_flyout ? "compact" : "full";
  if ($accounts.dataset.layout !== layout) { $accounts.replaceChildren(); $accounts.dataset.layout = layout; }
  if (layout === "compact") { renderCompactList(snap); return; }
  const groups = ["claude", "codex"]
    .map((p) => [p, snap.accounts.filter((a) => a.provider === p)])
    .filter(([, list]) => list.length);
  patch($accounts, groups.map(([provider, list]) => `
    <section>
      <div class="acc-head"><span class="logo ${provider}">${MARK[provider]}</span><span class="name">${PROVIDER_NAME[provider]}</span></div>
      ${list.map((a) => {
        const stale = a.stale && a.fetched_at
          ? t("lastObserved", { t: duration(Math.max(1, Math.round((Date.now() - new Date(a.fetched_at)) / 60000))) })
          : "";
        // Every row has one detail line, whatever its state, so switching doesn't change heights.
        // Where it's in use: here and on the user's other computers (shared via a synced folder).
        const places = placesText(a);
        const meta = [places.inUse || (a.alias ? a.email : ""), ...places.seen, a.leased ? t("leased", { purpose: a.leased }) : "", a.used_outside ? t("usedOutside") : "", a.org, stale]
          .filter(Boolean).join(" · ") || " ";
        return `
        <div class="acct ${a.active ? "active" : ""}" data-key="${provider}:${esc(a.id)}">
          <div class="acct-head">
            <span class="provider-mark ${provider}" title="${PROVIDER_NAME[provider]}">${MARK[provider]}</span>
            <span class="who">${esc(accountName(a))}</span>
            ${a.plan ? `<span class="plan">${esc(a.plan)}</span>` : ""}
            ${sparkline(trails[`${provider}:${a.id}`])}
            ${a.active
              ? `<span class="in-use ${a.pending ? "pending" : ""}">${t("inUse")}</span>`
              : `<button class="use" data-provider="${provider}" data-id="${esc(a.id)}">${t("use")}</button>`}
          </div>
          <div class="acc-meta" title="${esc([meta, a.leased ? t("leasedHint") : "", a.used_outside ? t("usedOutsideHint") : ""].filter(Boolean).join("\n"))}">${esc(meta)}</div>
          ${a.error || a.monitor_error ? `<div class="error">${esc(errorText(PROVIDER_NAME[provider], a.error ?? a.monitor_error))}</div>` : ""}
          ${!a.monitor_connected || ["monitor_reconnect", "monitor_storage", "monitor_wrong_account", "monitor_network", "monitor_uncertain"].includes(a.monitor_error ?? a.error)
            ? `<button class="monitor" data-provider="${provider}" data-id="${esc(a.id)}">${t(a.monitor_connected ? "monitorReconnect" : "monitorConnect")}</button>` : ""}
          ${a.monitor_connected ? `<button class="monitor" data-disconnect="true" data-provider="${provider}" data-id="${esc(a.id)}">${t("monitorDisconnect")}</button>` : ""}
          ${bars(a)}
          ${a.runs_out_at && !a.stale ? `<div class="reset runs-out" title="${esc(a.runs_out_at)}">${esc(t("runsOut", { at: dayTime(a.runs_out_at) }))}</div>` : ""}
          ${resetLines(a)}
          ${creditAdvice[`${provider}:${a.id}`] ? `<div class="reset-grant advice"><span>↺</span>${esc(creditAdvice[`${provider}:${a.id}`])}</div>` : ""}
          ${a.credits ? `<div class="reset" style="margin-top:6px">${esc(creditsText(a.credits))}</div>` : ""}
        </div>`;
      }).join("")}
    </section>`).join(""));
}

/** One line per account: provider mark, name, the binding window's %, the sparkline, and Use.
 * The row's tooltip says everything the full card would (accountTip), and screen readers get
 * the same text as the row's description. */
function renderCompactList(snap) {
  const list = ["claude", "codex"].flatMap((p) => snap.accounts.filter((a) => a.provider === p));
  patch($accounts, `<div class="compact-list" role="list" aria-label="${esc(t("title"))}">${list.map((a, i) => {
    const key = `${a.provider}:${a.id}`;
    const w = a.stale || a.error ? null : peak(a);
    const problem = a.error || a.monitor_error ? errorText(PROVIDER_NAME[a.provider], a.error ?? a.monitor_error) : "";
    const used = w ? `${Math.round(w.used_percent)}%` : "—";
    const usedText = w ? `${windowLabel(w)}: ${used} ${t("used")}` : t("usageUnknown");
    const tip = accountTip(a, { sub: subs[key], trail: trails[key], creditAdvice: creditAdvice[key] });
    return `
      <div class="acct line ${a.active ? "active" : ""}" role="listitem" data-key="${a.provider}:${esc(a.id)}" title="${esc(tip)}" aria-describedby="tip-${i}">
        <span class="sr-only" id="tip-${i}">${esc(tip)}</span>
        <span class="provider-mark ${a.provider}" role="img" aria-label="${PROVIDER_NAME[a.provider]}">${MARK[a.provider]}</span>
        <span class="who">${esc(accountName(a))}</span>
        ${problem ? `<span class="problem" role="img" aria-label="${esc(problem)}">!</span>` : ""}
        ${sparkline(trails[key], { tooltip: false })}
        <b class="pct" style="color:${w ? color(w.used_percent) : "var(--text-2)"}" aria-label="${esc(usedText)}">${used}</b>
        ${a.active
          ? `<span class="in-use ${a.pending ? "pending" : ""}">${t("inUse")}</span>`
          : `<button class="use" data-provider="${a.provider}" data-id="${esc(a.id)}" aria-label="${esc(`${t("use")} ${accountName(a)}`)}">${t("use")}</button>`}
      </div>`;
  }).join("")}</div>`);
}

/** Above 8 accounts the full cards make a long flyout: offer the compact layout once. */
function renderCompactOffer(snap) {
  const offer = document.getElementById("compact-offer");
  const show = !!current && !current.compact_flyout && !current.compact_suggested && snap.accounts.length > COMPACT_OFFER_ABOVE;
  offer.hidden = !show;
  if (show) document.getElementById("compact-offer-text").textContent = t("compactOffer", { n: snap.accounts.length });
}

async function answerCompactOffer(compact) {
  if (!current) return;
  const settings = { ...current, compact_suggested: true, compact_flyout: compact || current.compact_flyout };
  current = settings;
  render(lastSnap);
  await invoke("save_settings", { settings }).catch(() => {});
}
document.getElementById("compact-offer-yes").addEventListener("click", () => answerCompactOffer(true));
document.getElementById("compact-offer-no").addEventListener("click", () => answerCompactOffer(false));

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
  const monitor = e.target.closest("button.monitor");
  if (monitor) {
    monitor.disabled = true;
    try {
      const args = { provider: monitor.dataset.provider, id: monitor.dataset.id };
      if (monitor.dataset.disconnect) {
        // Reconnecting means a whole browser sign-in, so a stray click must not do this: the
        // first click asks, a second within a few seconds disconnects. (A dialog would take the
        // focus, and the flyout closes when it loses it.)
        if (!monitor.dataset.armed) {
          monitor.dataset.armed = "true";
          monitor.textContent = t("monitorDisconnectSure");
          monitor.classList.add("armed");
          setTimeout(() => { delete monitor.dataset.armed; monitor.classList.remove("armed"); monitor.textContent = t("monitorDisconnect"); }, 4000);
          return;
        }
        const result = await invoke("disconnect_monitoring", args);
        render(result.snapshot); showToast(t(result.revoked ? "monitorDisconnected" : "monitorRevokeFailed"));
      } else { await invoke("connect_monitoring", args); showToast(t("monitorPending")); }
    }
    catch (err) { showToast(errorText("", String(err))); }
    finally { monitor.disabled = false; }
    return;
  }
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
  // The account list scrolls once the flyout reaches the screen's height (the app caps the
  // window there), so ask for the height it would need unclipped.
  else invoke("set_panel_height", { height: Math.ceil($details.scrollHeight - $accounts.clientHeight + $accounts.scrollHeight) });
}

function render(snap) {
  lastSnap = snap;
  if (!IS_DOCK) loadCreditAdvice();
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

/** Both windows follow the taskbar's theme, like the shell's own flyouts; the backend says
 * when it changes (`taskbar-theme`), so nothing polls for it. */
const theme = () => invoke("taskbar_theme").then((t) => (document.documentElement.dataset.theme = t));
theme();
listen("taskbar-theme", ({ payload }) => (document.documentElement.dataset.theme = payload));

document.getElementById("open-settings").addEventListener("click", () => invoke("open_settings"));
document.getElementById("open-switcher").addEventListener("click", () => invoke("open_switcher"));
document.getElementById("toggle-compact").addEventListener("click", () => invoke("toggle_compact"));

function applySettings(settings) {
  current = settings;
  setLanguage(settings.language);
  translateDom();
  loadSubs();
  document.getElementById("toggle-compact").setAttribute("aria-pressed", String(!!settings.compact_flyout));
  if (IS_DOCK) {
    const labels = Object.fromEntries(["refresh", "settings", "quit"].map((k) => [k, t(k)]));
    labels.compact_flyout = t("compactFlyout");
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

listen("monitoring-login", ({ payload }) => showToast(payload.error ? errorText("", payload.error) : t("monitorConnected")));
listen("usage-updated", (e) => render(e.payload));
// The sparklines' readings: the backend records them on every refresh.
function loadTrails() {
  if (IS_DOCK) return;
  invoke("get_sparklines").then((found) => { trails = found ?? {}; render(lastSnap); }, () => {});
}
listen("usage-updated", loadTrails);
loadTrails();
// Each account's plan, renewal and verdict (get_subscriptions), keyed `provider:id`, for the
// compact rows' tooltips.
function loadSubs() {
  if (IS_DOCK || !current?.compact_flyout) return;
  invoke("get_subscriptions").then((report) => {
    subs = Object.fromEntries((report?.accounts ?? []).map((e) => [`${e.provider}:${e.id}`, e]));
    if (current?.compact_flyout) render(lastSnap);
  }, () => {});
}
listen("usage-updated", loadSubs);
loadSubs();
invoke("get_usage").then(render);
// Times ("resets in 2 h") move on between refreshes. The widget redraws every minute; the
// flyout only while it is open, and as it opens.
let panelOpen = false;
if (!IS_DOCK) {
  listen("panel-visible", ({ payload }) => {
    panelOpen = payload;
    if (payload) { invoke("get_usage").then(render); checkHealth(); }
  });
}
setInterval(() => { if (IS_DOCK || panelOpen) invoke("get_usage").then(render); }, 60_000);

// A warning sign in the flyout's header while any self-check (Settings > Health) has a problem.
const $health = document.getElementById("health-badge");
const $rosterWaiting = document.getElementById("roster-waiting");
async function checkHealth() {
  if (IS_DOCK) return;
  const report = await invoke("get_health").catch(() => null);
  $health.hidden = !report?.problems;
  if (report?.problems) { $health.title = t("healthIssues", { n: report.problems }); $health.setAttribute("aria-label", $health.title); }
  // Accounts other computers' rosters name that are not signed in here (Settings > Accounts signs them in).
  const waiting = report?.checks?.find((c) => c.id === "roster_waiting")?.count ?? 0;
  $rosterWaiting.hidden = !waiting;
  $rosterWaiting.textContent = waiting ? t("rosterWaiting", { n: waiting }) : "";
}
$health.addEventListener("click", () => invoke("open_settings", { page: "health" }));
$rosterWaiting.addEventListener("click", () => invoke("open_settings", { page: "accounts" }));
listen("usage-updated", checkHealth);
listen("monitoring-login", checkHealth);
checkHealth();

// Esc closes the flyout, as it does the shell's own.
if (!IS_DOCK) {
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); invoke("hide_panel"); }
  });
}

// Replay the flyout's entrance each time it opens.
if (!IS_DOCK) {
  window.addEventListener("focus", () => {
    $details.style.animation = "none";
    void $details.offsetWidth;
    $details.style.animation = "";
  });
}
