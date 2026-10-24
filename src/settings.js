import { MARK } from "./marks.js";
import { observeFrontend } from "./diagnostics.js";
import { sparkline } from "./sparkline.js";
import { money, pct, amount, renewalText, adviceText, creditsLine, extraUsageLine } from "./accounttext.js";
import { t, translateDom, setLanguage, setSystemLocale, LANGUAGES, resetText, errorText, duration, shortDate, shortDay, dayTime, windowLabel, windowResetText, visibleResetWindows, placesText } from "./i18n.js";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
observeFrontend("settings");
// Set until the first loads below have settled, so a capture (inspect_window) waits for them.
document.documentElement.dataset.loading = "1";

const REFRESH_OPTIONS = [1, 5, 15, 30];
const PROVIDERS = [["claude", "Claude"], ["codex", "Codex"]];
const $ = (id) => document.getElementById(id);
const esc = (v) => String(v ?? "").replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
const color = (p) => (p >= 90 ? "var(--bad)" : p >= 70 ? "var(--warn)" : "var(--ok)");

let settings;
let updatesEnabled = false;
let snapshot = { accounts: [] };
/** Each account's weekly usage over the last 7 days, keyed `provider:id` (get_sparklines). */
let trails = {};

// ---------------------------------------------------------------------------------------------
// Navigation

function showPage(page) {
  document.querySelectorAll(".nav").forEach((b) => {
    b.classList.toggle("active", b.dataset.page === page);
    if (b.dataset.page === page) b.setAttribute("aria-current", "page"); else b.removeAttribute("aria-current");
  });
  document.querySelectorAll(".page").forEach((p) => p.classList.toggle("active", p.id === `page-${page}`));
  try { localStorage.setItem("settings-page", page); } catch {}
}
document.querySelectorAll(".nav").forEach((b) => b.addEventListener("click", () => {
  showPage(b.dataset.page);
  if (b.dataset.page === "subscriptions") loadSubscriptions();
}));
// The navigation pane moves with the arrow keys, Home and End, like the Settings app's.
document.querySelector(".sidebar").addEventListener("keydown", (e) => {
  const navs = [...document.querySelectorAll(".nav")];
  const i = navs.indexOf(document.activeElement);
  if (i < 0) return;
  const next = { ArrowDown: i + 1, ArrowUp: i - 1, Home: 0, End: navs.length - 1 }[e.key];
  if (next == null) return;
  e.preventDefault();
  navs[(next + navs.length) % navs.length].focus();
});
// Esc leaves a text field being edited, and otherwise closes the window.
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || e.defaultPrevented) return;
  const el = document.activeElement;
  if (el && el.matches("input:not([type=checkbox]):not([type=range]), textarea")) { el.blur(); return; }
  window.__TAURI__.window.getCurrentWindow().close();
});

// ---------------------------------------------------------------------------------------------
// Accounts

const initial = (a) => (a.alias || a.email || "?").trim()[0].toUpperCase();
function hue(id) {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}
function accountRow(p, a) {
  const key = `${p}:${a.id}`;
  // The fullest window is the limit that binds next (and what the balancer compares), so it is
  // the headline; on a tie prefer the overall weekly window, whose reset lies further out.
  const top = a.windows.reduce((m, w) => (!m || w.used_percent > m.used_percent
    || (w.used_percent === m.used_percent && w.kind === "weekly") ? w : m), null);
  const usage = top?.used_percent ?? null;
  const timedWindows = visibleResetWindows(a.windows);
  const meter = usage == null
    ? `<div class="meter none"><b>—</b></div>`
    : `<div class="meter ${a.stale ? "stale" : ""}" title="${esc(a.windows.map(w => `${windowLabel(w)}: ${w.used_percent}% ${t("used")}${w.resets_at ? ` · ${w.resets_at}` : ""}`).join("\n"))}"><b>${Math.round(usage)}%</b><small class="window-label">${esc(windowLabel(top))} · ${t("used")}</small><div class="track" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(Math.min(100, usage))}" aria-label="${esc(`${windowLabel(top)} ${t("used")}`)}"><i style="width:${Math.min(100, usage)}%;background:${color(usage)}"></i></div>
        ${sparkline(trails[key], { width: 140, height: 18 })}
        ${a.stale ? `<small>${t("lastReading")}</small>` : ""}</div>`;
  // A zero-usage session can lack a reset while its weekly window has one.
  // Never hide that weekly timestamp behind the headline's tie-breaking.
  const windowResets = timedWindows.length
    ? `<div class="window-resets">${timedWindows.map(w => `<small class="window-reset" title="${esc(w.resets_at)}">${esc(windowLabel(w))}: ${Math.round(w.used_percent)}% ${t("used")} · ${esc(windowResetText(w, a.stale, a.fetched_at))}</small>`).join("")}</div>`
    : "";
  // In use here and on the user's other computers (shared through a synced folder).
  const places = placesText(a);
  const held = a.resets ?? [];
  const total = held.reduce((n, r) => n + r.count, 0);
  const resets = total > 0
    ? `<em class="chip resets ${held.some((r) => r.usable) ? "live" : ""}" aria-label="${esc(t("resetsHeld", { n: total }))}"
        title="${esc([t("resetsTitle"), ...held.map(resetText)].join("\n"))}">↺ ${total}</em>`
    : "";
  return `
    <div class="account" data-key="${p}:${esc(a.id)}">
      <div class="avatar" style="background:hsl(${hue(a.id)} 55% 48%)">${esc(initial(a))}</div>
      <div class="who">
        <input data-alias="${esc(key)}" value="${esc(settings.aliases[key] ?? "")}"
          placeholder="${esc(a.email ?? t("unknownAccount"))}" aria-label="${t("alias")}" title="${t("alias")}" />
        <small>
          ${a.active ? `<em class="chip live" title="${esc(places.inUse)}">${t("inUse")}</em>`
            : places.elsewhere ? `<em class="chip elsewhere" title="${esc(places.inUse)}">${esc(places.inUse)}</em>` : ""}
          ${a.used_outside ? `<em class="chip outside" title="${esc(t("usedOutsideHint"))}">${t("usedOutside")}</em>` : ""}
          ${a.unexpected ? `<em class="chip unexpected" title="${esc(t("unexpectedHint", { t: duration(Math.max(1, Math.round((Date.now() - new Date(a.unexpected.last_seen)) / 60000))) }))}">${esc(t(a.unexpected.seen_week === 1 ? "unexpectedSeenOnce" : "unexpectedSeen", { n: a.unexpected.seen_week }))}</em>` : ""}
          ${a.plan ? `<em class="chip">${esc(a.plan)}</em>` : ""}
          ${signInChips(p, a)}
          ${resets}
          <span>${esc([a.active && places.elsewhere ? places.inUse : "", ...places.seen, settings.aliases[key] ? a.email ?? "" : a.org ?? ""].filter(Boolean).join(" · "))}</span>
        </small>
      </div>
      <div class="monitor-state">
        ${windowResets}
        ${a.stale && a.fetched_at ? `<small>${esc(t("lastObserved", { t: duration(Math.max(1, Math.round((Date.now() - new Date(a.fetched_at)) / 60000))) }))}</small>` : ""}
        ${a.error || a.monitor_error ? `<small>${esc(errorText(p, a.error ?? a.monitor_error))}</small>` : ""}
        ${!a.monitor_connected || ["monitor_reconnect", "monitor_storage", "monitor_wrong_account", "monitor_network", "monitor_uncertain"].includes(a.monitor_error ?? a.error)
          ? `<button class="btn" data-monitor="${p}" data-id="${esc(a.id)}">${t(a.monitor_connected ? "monitorReconnect" : "monitorConnect")}</button>`
          : ""}
        ${a.monitor_connected ? `<button class="btn" data-disconnect="${p}" data-id="${esc(a.id)}">${t("monitorDisconnect")}</button>` : ""}
        <button class="btn" data-revoke-all="${p}" data-id="${esc(a.id)}" data-active="${a.active ? "1" : ""}" title="${esc(t("revokeAllHint"))}">${t("revokeAll")}</button>
        ${a.monitor_connected && (settings.presence_folder || settings.presence_remote)
          ? `<label class="web-use" title="${esc(t("webUseHint"))}"><input type="checkbox" data-web-use="${esc(key)}" ${settings.web_use?.[key] ? "checked" : ""} /> ${t("webUse")}</label>`
          : ""}
      </div>
      ${meter}
      ${a.active
        // Holds the button's place so every row's meter lines up.
        ? `<span class="btn use" aria-hidden="true" style="visibility:hidden">${t("use")}</span>`
        : `<button class="btn use" data-use="${p}" data-id="${esc(a.id)}" aria-label="${esc(t("onAccount", { action: t("use"), name: a.alias || a.email || t("unknownAccount") }))}">${t("use")}</button>`}
      <button class="remove" data-remove="${p}" data-id="${esc(a.id)}" ${a.active ? "disabled" : ""} title="${t("remove")}" aria-label="${esc(t("removeAccountNamed", { name: a.alias || a.email || t("unknownAccount") }))}">
        <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8"/></svg>
      </button>
    </div>`;
}

/** Sign-in health: the provider rejected the monitoring sign-in, or the CLI's own saved login. */
function signInChips(p, a) {
  const monitoring = a.monitor_connected && ["monitor_reconnect", "monitor_wrong_account", "monitor_uncertain"].includes(a.monitor_error ?? a.error);
  const cli = p === "claude" ? "claude /login" : "codex login";
  return [
    monitoring ? `<em class="chip unexpected" title="${esc(t("monitorRejectedHint"))}">${t("monitorRejected")}</em>` : "",
    a.cli_rejected ? `<em class="chip unexpected" title="${esc(t("cliRejectedHint", { cli }))}">${t("cliRejected")}</em>` : "",
  ].join("");
}

/** Replaces only the account rows that changed (see main.js), so buttons don't flicker. */
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
  // Balancer controls live outside the rows; keep them in sync too.
  container.querySelectorAll(".balancer").forEach((el, i) => {
    const twin = next.querySelectorAll(".balancer")[i];
    if (twin && twin.outerHTML !== el.outerHTML) el.replaceWith(twin);
  });
}

function renderAccounts() {
  patch($("accounts"), PROVIDERS.filter(([p]) => settings.providers[p]).map(([p, name]) => {
    const list = snapshot.accounts.filter((a) => a.provider === p);
    const rule = settings.balancer[p];
    return `
      <div class="provider">
        <div class="provider-head">
          <span class="logo ${p}">${MARK[p]}</span><h3>${name}</h3>
          <button class="btn" data-add="${p}"><svg viewBox="0 0 16 16"><path d="M8 3v10M3 8h10"/></svg>${t("addAccount")}</button>
        </div>
        <div class="group">
          ${list.length ? list.map((a) => accountRow(p, a)).join("") : `<div class="empty">${t("noAccounts")}</div>`}
        </div>
        <div class="group balancer">
          <div class="row">
            <label class="title" for="auto-${p}">${t("autoSwitch")}</label>
            <input type="checkbox" class="switch" role="switch" id="auto-${p}" data-auto="${p}" ${rule.auto ? "checked" : ""} />
          </div>
          <div class="row ${rule.auto ? "" : "disabled"}">
            <label for="at-${p}">${t("switchAt")}</label>
            <div class="range">
              <input type="range" id="at-${p}" data-threshold="${p}" min="50" max="95" step="5" value="${rule.threshold}"
                style="--fill:${((rule.threshold - 50) / 45) * 100}%" />
              <output>${rule.threshold}%</output>
            </div>
          </div>
        </div>
      </div>`;
  }).join(""));
}

$("accounts").addEventListener("click", async (e) => {
  const disconnect = e.target.closest("[data-disconnect]");
  if (disconnect) {
    disconnect.disabled = true;
    try {
      const result = await invoke("disconnect_monitoring", { provider: disconnect.dataset.disconnect, id: disconnect.dataset.id });
      snapshot = result.snapshot; renderAccounts();
      $("add-hint").textContent = t(result.revoked ? "monitorDisconnected" : "monitorRevokeFailed"); $("add-hint").hidden = false;
    } catch (err) { alert(errorText("", String(err))); }
    finally { disconnect.disabled = false; }
    return;
  }
  const revokeAll = e.target.closest("[data-revoke-all]");
  if (revokeAll) {
    const { revokeAll: provider, id, active } = revokeAll.dataset;
    const cli = provider === "claude" ? "claude /logout" : "codex logout";
    if (!confirm(t("revokeAllConfirm", { cli }))) return;
    // The saved CLI login is only offered for removal when the account is not in use.
    const removeSlot = !active && confirm(t("revokeAllRemoveSlot"));
    revokeAll.disabled = true;
    try {
      const result = await invoke("revoke_account_sessions", { provider, id, removeSlot });
      snapshot = result.snapshot; renderAccounts();
      const done = { revoked: "revokeAllRevoked", unconfirmed: "monitorRevokeFailed", none: "revokeAllNone" }[result.monitoring];
      $("add-hint").textContent = [t(done), result.slot_removed ? t("revokeAllSlotRemoved") : t("revokeAllCliManaged", { cli })].join(" ");
      $("add-hint").hidden = false;
    } catch (err) { alert(err === "active" ? t("activeCantRemove") : errorText("", String(err))); }
    finally { revokeAll.disabled = false; }
    return;
  }
  const monitor = e.target.closest("[data-monitor]");
  if (monitor) {
    monitor.disabled = true;
    try {
      await invoke("connect_monitoring", { provider: monitor.dataset.monitor, id: monitor.dataset.id });
      $("add-hint").textContent = t("monitorPending"); $("add-hint").hidden = false;
    } catch (err) { alert(errorText("", String(err))); }
    finally { monitor.disabled = false; }
    return;
  }
  const add = e.target.closest("[data-add]");
  const remove = e.target.closest("[data-remove]");
  const use = e.target.closest("[data-use]");
  if (use) {
    const { use: provider, id } = use.dataset;
    const before = snapshot;
    // Mark the new account at once; the backend confirms (or reverts) it.
    snapshot = { ...snapshot, accounts: snapshot.accounts.map((a) => (a.provider === provider ? { ...a, active: a.id === id } : a)) };
    renderAccounts();
    try {
      snapshot = await invoke("switch_account", { provider, id });
    } catch (err) {
      snapshot = before;
      alert(String(err));
    }
    renderAccounts();
  } else if (add) {
    try {
      await invoke("add_account", { provider: add.dataset.add });
      $("add-hint").hidden = false;
    } catch (err) {
      alert(err === "CLI not found" ? t("cliMissing") : String(err));
    }
  } else if (remove && !remove.disabled) {
    snapshot = await invoke("remove_account", { provider: remove.dataset.remove, id: remove.dataset.id });
    renderAccounts();
  }
});
$("accounts").addEventListener("input", (e) => {
  const el = e.target;
  if (!el.dataset.threshold) return;
  el.style.setProperty("--fill", `${((el.value - 50) / 45) * 100}%`);
  el.nextElementSibling.textContent = `${el.value}%`;
});
$("accounts").addEventListener("change", (e) => {
  const el = e.target;
  if (el.dataset.alias) {
    const aliases = { ...settings.aliases };
    const value = el.value.trim();
    if (value) aliases[el.dataset.alias] = value;
    else delete aliases[el.dataset.alias];
    update({ aliases });
  } else if (el.dataset.webUse) {
    const web_use = { ...settings.web_use };
    if (el.checked) web_use[el.dataset.webUse] = true;
    else delete web_use[el.dataset.webUse];
    update({ web_use });
  } else if (el.dataset.auto) {
    const p = el.dataset.auto;
    update({ balancer: { ...settings.balancer, [p]: { ...settings.balancer[p], auto: el.checked } } });
  } else if (el.dataset.threshold) {
    const p = el.dataset.threshold;
    update({ balancer: { ...settings.balancer, [p]: { ...settings.balancer[p], threshold: Number(el.value) } } });
  }
});
$("accounts").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.dataset.alias) e.target.blur();
});
listen("usage-updated", (e) => {
  snapshot = e.payload;
  // Don't rebuild the list under someone typing a nickname.
  if (settings && !document.activeElement?.dataset?.alias) renderAccounts();
  if (settings) renderGuide();
  loadTrails();
});
async function loadTrails() {
  trails = await invoke("get_sparklines").catch(() => trails) ?? {};
  if (settings && !document.activeElement?.dataset?.alias) renderAccounts();
}
listen("monitoring-login", ({ payload }) => {
  $("add-hint").textContent = payload.error ? errorText("", payload.error) : t("monitorConnected");
  $("add-hint").hidden = false;
});
listen("account-added", () => { $("add-hint").hidden = true; });

// ---------------------------------------------------------------------------------------------
// Subscriptions: which paid plans run out every week and which sit idle, and when they renew.
// The verdicts come from the backend (subscriptions.rs); this page only words them.

let report = null;
const NOTIFY_KINDS = ["usage", "resets", "switches", "renewals", "extra_usage", "burn", "reset_credits", "unexpected_use", "digest"];
const VERDICT_TONE = { exhausted_weekly: "bad", unused: "warn", underused: "warn", healthy: "ok", insufficient_data: "" };

function verdictLines(e) {
  const a = e.assessment;
  const lines = [];
  if (a.verdict === "insufficient_data") {
    lines.push(t("subInsufficient", { n: a.weeks_counted, needed: a.weeks_needed, days: e.days_tracked }));
    return lines;
  }
  lines.push(t("subRecent", { hit: a.limit_weeks, n: a.weeks_counted, avg: pct(a.average_peak ?? 0) }));
  if (a.sub_window_limited) lines.push(t("subModelCap", { window: windowLabel({ kind: a.sub_window_limited }) }));
  if (a.basis === "local_codex") lines.push(t(a.weeks_counted ? "subLocalWeeks" : "subLocalBasis"));
  else if (a.confidence === "low") lines.push(t("subLowConfidence"));
  return lines;
}

const compact = (v) => new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(v);

// What the Codex CLI on this PC remembers: "Last used here: Oct 8 · 12 days idle · 0 threads in 28 days".
function localText(e) {
  const l = e.local;
  if (!l) return "";
  const parts = [l.never_seen ? t("subLocalNever", { d: shortDate(l.attributed_since) })
    : l.days_idle === 0 ? t("subLocalToday") : t("subLocalLast", { d: shortDate(l.last_local_activity) })];
  if (!l.never_seen && l.days_idle > 0) parts.push(t("subLocalIdle", { n: l.days_idle }));
  parts.push(t("subLocalThreads", { n: l.threads_28d }));
  if (l.tokens_28d) parts.push(t("subLocalTokens", { v: compact(l.tokens_28d) }));
  return parts.join(" · ");
}

// What the plan costs for the use it gets: "$0.92 per 1% of the weekly limit used · $12.50 per thread (this PC only)".
function costPerUseText(e) {
  const c = e.cost_per_use;
  if (!c) return "";
  const parts = [];
  if (c.per_percent != null) parts.push(t("subPerPercent", { v: money(c.per_percent) }));
  if (c.per_local_thread != null) parts.push(t("subPerThread", { v: money(c.per_local_thread) }));
  return parts.join(" · ");
}

function subscriptionRow(e) {
  const key = `${e.provider}:${e.id}`;
  const prefs = settings.subscriptions?.[key] ?? {};
  const a = e.assessment;
  const week = e.this_week;
  const weekText = week ? t(week.pace_percent != null && week.pace_percent > 100 && week.used_percent < 100 ? "subThisWeekFast" : "subThisWeek",
    { p: pct(week.used_percent), t: week.resets_at ? duration(Math.max(1, Math.round((new Date(week.resets_at) - Date.now()) / 60000))) : "—" }) : t("subNoWeekly");
  const paid = e.plan_kind === "personal";
  const kindNote = e.plan_kind === "organization" ? t("subOrgManaged") : e.plan_kind === "free" ? t("subFree") : e.plan_kind === "unknown" ? t("subPlanUnknown") : "";
  const cost = e.monthly_cost != null ? t(e.cost_entered ? "subCostEntered" : "subCostList", { v: money(e.monthly_cost) }) : paid ? t("subCostUnknown") : "";
  const providerDate = e.provider_until && e.renewal?.source !== "provider" && new Date(e.provider_until) < Date.now()
    ? t("subProviderStale", { d: shortDate(e.provider_until) }) : "";
  const advice = adviceText(e);
  const local = localText(e);
  const perUse = costPerUseText(e);
  const credits = creditsLine(e.credits);
  const extra = extraUsageLine(e.extra_usage);
  // Anything resting on this PC's history says what it cannot see.
  const caveat = e.local && (a.basis === "local_codex" || a.verdict === "insufficient_data") ? t("subLocalCaveat") : "";
  return `
    <div class="account sub" data-key="${esc(key)}">
      <div class="avatar" style="background:hsl(${hue(e.id)} 55% 48%)">${esc((e.alias || e.email || "?").trim()[0].toUpperCase())}</div>
      <div class="who">
        <div class="name">${esc(e.alias || e.email || t("unknownAccount"))}</div>
        <small>
          ${e.active ? `<em class="chip live">${t("inUse")}</em>` : ""}
          ${e.plan ? `<em class="chip">${esc(e.plan)}</em>` : ""}
          ${e.ends_at ? `<em class="chip">${t("chipCancelled")}</em>` : e.cancel_after ? `<em class="chip">${t("chipLapsing")}</em>` : ""}
          <em class="chip verdict ${VERDICT_TONE[a.verdict]}">${t(`subVerdict_${a.verdict}`)}</em>
        </small>
      </div>
      <div class="meter">${cost ? `<b>${esc(cost)}</b>` : ""}${paid ? `<small>${esc(renewalText(e))}</small>` : ""}</div>
      ${paid ? `<button class="btn use" data-billing="${e.provider}">${t("subManage")} <svg viewBox="0 0 16 16"><path d="M6 3h7v7M13 3 5 11"/></svg></button>`
        : `<span class="btn use" aria-hidden="true" style="visibility:hidden">${t("subManage")}</span>`}
      <span></span>
      <div class="monitor-state">
        <small>${esc(weekText)}${e.stale ? ` · ${t("lastReading")}` : ""}${week?.runs_out_at ? ` · <b class="runs-out">${esc(t("runsOut", { at: dayTime(week.runs_out_at) }))}</b>` : ""}</small>
        ${local ? `<small>${esc(local)}</small>` : ""}
        ${perUse ? `<small>${esc(perUse)}</small>` : ""}
        ${credits ? `<small>${esc(credits)}</small>` : ""}
        ${extra ? `<small class="${e.extra_usage.utilization >= 75 ? "advice" : ""}">${esc(extra)}</small>` : ""}
        ${e.idle_now ? `<small>${t("subIdleNow")}</small>` : ""}
        ${e.allowance_change?.would_hit_limit ? `<small class="advice">${esc(t("subProCut", { d: shortDate(`${e.allowance_change.last_day}T12:00:00`), p: Math.round(e.allowance_change.peak_percent), q: Math.round(e.allowance_change.projected_percent) }))}</small>` : ""}
        ${verdictLines(e).map((l) => `<small>${esc(l)}</small>`).join("")}
        ${providerDate ? `<small>${esc(providerDate)}</small>` : ""}
        ${kindNote ? `<small>${esc(kindNote)}</small>` : ""}
        ${e.store_billed ? `<small>${t("subStoreBilled")}</small>` : ""}
        ${e.delinquent ? `<small class="advice">${t("subDelinquent")}</small>` : ""}
        ${advice ? `<small class="advice">${esc(advice)}</small>` : ""}
        ${caveat ? `<small>${esc(caveat)}</small>` : ""}
        ${paid ? `<div class="plan-fields">
          <label>${t("subNextBilling")} <input type="date" data-sub-date="${esc(key)}" value="${e.renewal ? e.renewal.at.slice(0, 10) : ""}" title="${t("subCorrectHint")}" /></label>
          ${prefs.next_billing || prefs.billing_day ? `<button class="link" data-sub-reset="${esc(key)}">${t("subResetDate")}</button>` : ""}
          <label>${t("subMonthlyCost")} <input type="number" min="0" max="10000" step="0.01" data-sub-cost="${esc(key)}" value="${prefs.monthly_cost ?? ""}" placeholder="${e.monthly_cost ?? "—"}" /></label>
          <label class="lapse"><input type="checkbox" data-sub-lapse="${esc(key)}" data-until="${esc(e.renewal?.at ?? "")}" ${e.cancel_after ? "checked" : ""} /> ${t("subLettingLapse")}</label>
        </div>` : ""}
      </div>
    </div>`;
}

function providerSummary(p, s) {
  if (!s || !s.paid) return "";
  const parts = [t("subPaidCount", { n: s.paid })];
  if (s.monthly_total) parts.push(t("subMonthlyTotal", { v: money(s.monthly_total) }) + (s.unpriced ? ` + ${t("subUnpriced", { n: s.unpriced })}` : ""));
  if (s.exhausted) parts.push(t("subExhaustedCount", { n: s.exhausted }));
  if (s.unused) parts.push(t("subUnusedCount", { n: s.unused }));
  if (s.possible_savings) parts.push(t("subSavings", { v: money(s.possible_savings) }));
  if (s.extra_usage_spent != null) parts.push(t("subExtraUsageTotal", { v: money(s.extra_usage_spent) }));
  return parts.join(" · ");
}

// Pool sizing: how many plans the provider's usage needs, against how many are held.
function poolText(s) {
  const p = s?.pool;
  if (!s?.paid || !p || !p.plans_held) return "";
  const parts = [];
  if (p.plans_needed == null) parts.push(t("subPoolInsufficient", { n: p.weeks_used, needed: p.weeks_needed }));
  else {
    parts.push(t("subPool", { n: p.plans_needed, unit: p.unit, spare: pct(p.spare_percent ?? 0), held: amount(p.plans_held) }));
    parts.push(t("subPoolBasis", { n: p.weeks_used }) + (p.confidence === "low" ? ` (${t("subPoolEstimate")})` : ""));
    if (p.basis === "local_codex") parts.push(t("subPoolLocal"));
  }
  if (p.unweighted) parts.push(t("subPoolUnweighted", { n: p.unweighted }));
  const c = s.cost_per_use;
  if (c?.per_plan_percent != null) parts.push(t("subBlendedPercent", { v: money(c.per_plan_percent), unit: p.unit }));
  if (c?.per_local_thread != null) parts.push(t("subPerThread", { v: money(c.per_local_thread) }));
  if (s.credit_balance != null) parts.push(t("subCredits", { v: amount(s.credit_balance) }));
  return parts.join(" · ");
}

const PROVIDER_NAMES = Object.fromEntries(PROVIDERS);

// The plan change advisor: concrete moves per provider, with what they save and what is left.
function adviceMove(m) {
  const who = m.alias || m.email || t("unknownAccount");
  const what = m.action === "cancel" ? t("advCancel", { plan: m.from }) : t("advDowngrade", { from: m.from, to: m.to });
  const why = [t(`subVerdict_${m.verdict}`), m.peak_percent != null ? t("advPeak", { p: pct(m.peak_percent) }) : "",
    m.basis === "local_codex" ? t("advLocal") : "", m.before ? t("advBefore", { d: shortDate(m.before) }) : ""].filter(Boolean).join(" · ");
  return `
    <div class="advice-move">
      <div class="who"><div class="name">${esc(who)}</div><small>${esc(what)} · <b>${esc(t("perMonth", { v: money(m.monthly_saving) }))}</b></small><small>${esc(why)}</small></div>
      <button class="btn use" data-billing="${m.provider}">${t("subManage")} <svg viewBox="0 0 16 16"><path d="M6 3h7v7M13 3 5 11"/></svg></button>
    </div>`;
}

function adviceCard() {
  const list = (report.advice ?? []).filter((a) => settings.providers[a.provider]);
  if (!list.length) return "";
  return `
    <div class="provider advisor">
      <div class="provider-head"><h3>${t("advTitle")}</h3></div>
      ${list.map((a) => {
        const name = PROVIDER_NAMES[a.provider] ?? a.provider;
        const lines = [];
        if (a.evidence === "clear_only") lines.push(t("advThin", { n: a.weeks_used, needed: a.weeks_needed }));
        const head = a.moves.length ? t("advSave", { provider: name, v: money(a.monthly_saving), n: a.moves.length }) : t("advNone", { provider: name });
        if (a.spare_after != null) lines.push(t("advAfter", { held: amount(a.capacity_after), unit: a.unit, spare: a.spare_after_percent != null ? pct(a.spare_after_percent) : "—" }));
        if (a.kept_for_capacity) lines.push(t("advKept", { n: a.kept_for_capacity }));
        return `
          <p class="hint sub-pool"><b>${esc(head)}</b>${lines.length ? ` · ${esc(lines.join(" · "))}` : ""}</p>
          ${a.moves.length ? `<div class="group">${a.moves.map(adviceMove).join("")}</div>` : ""}`;
      }).join("")}
      <p class="hint">${t("advHint")}</p>
    </div>`;
}

// The spend ledger: what the plans cost, year to date and over the last twelve months.
function spendCard() {
  const s = report.spend;
  if (!s) return "";
  const label = s.currency ? ` ${s.currency}` : "";
  const taxed = (v, w) => esc(money(v) + label) + (w != null ? ` <small>(${esc(t("spendWithTax", { v: money(w), p: s.tax_percent }))})</small>` : "");
  const months = s.months.slice(-6).map((m) => `${m.month}: ${money(m.total)}${m.extra_usage ? ` (${t("spendExtra", { v: money(m.extra_usage) })})` : ""}`).join(" · ");
  const prefs = settings.spend ?? {};
  return `
    <div class="provider spend">
      <div class="provider-head"><h3>${t("spendTitle")}</h3></div>
      <p class="hint sub-pool">${t("spendYtd")}: <b>${taxed(s.year_to_date, s.year_to_date_with_tax)}</b> · ${t("spend12")}: <b>${taxed(s.trailing_12_months, s.trailing_12_months_with_tax)}</b></p>
      ${months ? `<p class="hint sub-pool">${esc(months)}</p>` : `<p class="hint sub-pool">${t("spendEmpty")}</p>`}
      <div class="plan-fields">
        <label>${t("spendTax")} <input type="number" min="0" max="100" step="0.01" data-spend-tax value="${prefs.tax_percent ?? ""}" placeholder="0" /></label>
        <label>${t("spendCurrency")} <input type="text" maxlength="24" data-spend-currency value="${esc(prefs.currency ?? "")}" placeholder="USD" /></label>
      </div>
      <p class="hint">${t("spendHint")}</p>
    </div>`;
}

function renderSubscriptions() {
  if (!report || !settings) return;
  patch($("subscriptions"), adviceCard() + PROVIDERS.filter(([p]) => settings.providers[p]).map(([p, name]) => {
    const list = report.accounts.filter((e) => e.provider === p);
    const totals = report.providers.find((s) => s.provider === p);
    const summary = providerSummary(p, totals);
    const pool = poolText(totals);
    const since = list.find((e) => e.local)?.local.attributed_since;
    return `
      <div class="provider">
        <div class="provider-head">
          <span class="logo ${p}">${MARK[p]}</span><h3>${name}</h3>
          ${summary ? `<small class="sub-summary">${esc(summary)}</small>` : ""}
        </div>
        ${pool ? `<p class="hint sub-pool">${esc(pool)}</p>` : ""}
        <div class="group">
          ${list.length ? list.map(subscriptionRow).join("") : `<div class="empty">${t("noAccounts")}</div>`}
        </div>
        <p class="hint">${t(p === "claude" ? "subClaudeFree" : "subCodexFree")}</p>
        ${p === "codex" && since ? `<p class="hint">${esc(t("subCodexLocal", { d: shortDate(since) }))}</p>` : ""}
      </div>`;
  }).join("") + spendCard());
}

async function loadSubscriptions() {
  try { report = await invoke("get_subscriptions"); } catch { return; }
  // Don't rebuild the rows under someone typing a billing day or cost.
  if (!editingPlan()) renderSubscriptions();
}

const editingPlan = () => !!document.activeElement?.closest?.(".plan-fields");

function setSpend(change) {
  const spend = { ...(settings.spend ?? {}), ...change };
  return update({ spend }).then(loadSubscriptions);
}

function setPlan(key, change) {
  // Show the change at once; the report reloads with the backend's view of it.
  const entry = report?.accounts.find((e) => `${e.provider}:${e.id}` === key);
  if (entry && "cancel_after" in change) entry.cancel_after = change.cancel_after;
  const next = { ...(settings.subscriptions?.[key] ?? {}), ...change };
  for (const k of Object.keys(next)) if (next[k] == null || next[k] === "") delete next[k];
  const subscriptions = { ...settings.subscriptions };
  if (Object.keys(next).length) subscriptions[key] = next; else delete subscriptions[key];
  return update({ subscriptions }).then(loadSubscriptions);
}

$("subscriptions").addEventListener("click", async (e) => {
  const reset = e.target.closest("[data-sub-reset]");
  if (reset) { setPlan(reset.dataset.subReset, { next_billing: null, billing_day: null }); return; }
  const billing = e.target.closest("[data-billing]");
  if (!billing) return;
  try { await invoke("open_billing_page", { provider: billing.dataset.billing }); } catch (err) { alert(String(err)); }
});
$("subscriptions").addEventListener("change", (e) => {
  const el = e.target;
  if (el.dataset.subDate != null) {
    // A corrected date replaces any older billing-day override.
    setPlan(el.dataset.subDate, { next_billing: /^\d{4}-\d{2}-\d{2}$/.test(el.value) ? el.value : null, billing_day: null });
  } else if (el.dataset.subCost != null) {
    const cost = Number(el.value);
    setPlan(el.dataset.subCost, { monthly_cost: el.value && cost >= 0 && cost <= 10000 ? Math.round(cost * 100) / 100 : null });
  } else if (el.dataset.spendTax != null) {
    const v = Number(el.value);
    setSpend({ tax_percent: el.value && v > 0 && v <= 100 ? Math.round(v * 100) / 100 : null });
  } else if (el.dataset.spendCurrency != null) {
    setSpend({ currency: el.value.trim().slice(0, 24) || null });
  } else if (el.dataset.subLapse != null) {
    // The last paid day is the day before renewal; without a known renewal, keep it in rotation a month.
    const until = el.dataset.until ? new Date(new Date(el.dataset.until).getTime() - 86400000) : new Date(Date.now() + 30 * 86400000);
    setPlan(el.dataset.subLapse, { cancel_after: el.checked ? until.toISOString().slice(0, 10) : null });
  }
});
// Rows skipped while a plan was being edited catch up once focus leaves the fields.
$("subscriptions").addEventListener("focusout", () => setTimeout(() => { if (!editingPlan()) renderSubscriptions(); }));
listen("usage-updated", loadSubscriptions);
// A plan notification was clicked.
listen("show-page", ({ payload: page }) => document.querySelector(`.nav[data-page="${page}"]`)?.click());

// ---------------------------------------------------------------------------------------------
// Usage: the backend's index of the CLIs' files (usage_index.rs), by repository and by model.

const USAGE_DAYS = [7, 28];
let usage = null;
let usageDays = 7;
const heatPick = {};
try { usageDays = Number(localStorage.getItem("usage-days")) || usageDays; } catch {}

const accountName = (s) => s.alias || s.email || (s.id ? t("usageUnknown") : t("usageUnattributed"));

function bars(groups, limit = 5) {
  if (!groups?.length) return `<div class="usage-empty">${t("usageNone")}</div>`;
  return `<ol class="bars">${groups.slice(0, limit).map((g) => `
    <li title="${esc(t("usageSplit", { i: compact(g.tokens.input + g.tokens.cache_write), o: compact(g.tokens.output), c: compact(g.tokens.cached) }))}">
      <span class="bar-key">${esc(g.key)}</span><span class="bar-value">${compact(g.tokens.total)} · ${g.share}%</span>
      <span class="bar" style="--w:${Math.max(1, g.share)}%"></span>
    </li>`).join("")}</ol>`;
}

function weekdayNames() {
  // 2024-01-01 was a Monday.
  return [...Array(7)].map((_, i) => new Date(2024, 0, 1 + i).toLocaleDateString(undefined, { weekday: "short" }));
}

function heatmap(grid) {
  const max = Math.max(1, ...grid.flat());
  const days = weekdayNames();
  const hours = [...Array(24).keys()];
  return `<div class="heat" role="img" aria-label="${esc(t("usageHeatmap"))}">
    <span></span>${hours.map((h) => `<span class="heat-hour">${h % 6 === 0 ? h : ""}</span>`).join("")}
    ${grid.map((row, d) => `<span class="heat-day">${esc(days[d])}</span>${row.map((v, h) =>
      `<span class="heat-cell" style="--a:${v ? 0.12 + 0.88 * (v / max) : 0}" title="${esc(`${days[d]} ${h}:00 · ${compact(v)}`)}"></span>`).join("")}`).join("")}
  </div>`;
}

function usageAccount(s, models) {
  const note = s.id == null ? t("usageUnattributedHint") : !s.known ? t("usageUnknown")
    : s.attribution === "inferred_from_active_account" ? t("usageInferred") : s.attribution === "mixed" ? t("usageMixed") : "";
  const tok = s.tokens;
  return `
    <div class="row usage-account" data-key="${esc(`${s.provider}:${s.id ?? ""}`)}">
      <div class="usage-who">
        <label>${esc(accountName(s))}</label>
        <small>${tok.total ? esc(`${t("usageTotal", { v: compact(tok.total) })} · ${t("usageSplit", { i: compact(tok.input + tok.cache_write), o: compact(tok.output), c: compact(tok.cached) })}`) : esc(t("usageNone"))}</small>
        ${note && tok.total ? `<small class="usage-note">${esc(note)}</small>` : ""}
      </div>
      ${tok.total ? `<div class="usage-cols">
        <div><h4>${t("usageTopRepos")}</h4>${bars(s.groups)}</div>
        <div><h4>${t("usageTopModels")}</h4>${bars(models?.groups)}</div>
      </div>` : ""}
    </div>`;
}

function renderUsage() {
  if (!settings) return;
  const box = $("usage");
  if (!usage) { box.innerHTML = `<p class="hint">${t("usageLoading")}</p>`; return; }
  const [repos, models] = usage;
  const slice = (b, s) => b.accounts.find((x) => x.provider === s.provider && x.id === s.id);
  box.innerHTML = PROVIDERS.filter(([p]) => settings.providers[p]).map(([p, name]) => {
    const total = repos.providers.find((x) => x.provider === p);
    const accounts = repos.accounts.filter((s) => s.provider === p)
      .sort((a, b) => (a.id == null) - (b.id == null) || b.tokens.total - a.tokens.total);
    const pick = heatPick[p] ?? "";
    const heatFrom = pick ? accounts.find((s) => (s.id ?? "-") === pick) : total;
    const summary = [t("usageTotal", { v: compact(total?.tokens.total ?? 0) })];
    if (total?.unattributed.total) summary.push(t("usageUnattributedTotal", { v: compact(total.unattributed.total) }));
    return `
      <div class="provider">
        <div class="provider-head"><span class="logo ${p}">${MARK[p]}</span><h3>${name}</h3><small class="sub-summary">${esc(summary.join(" · "))}</small></div>
        ${total?.tokens.total ? `<div class="row usage-overview">
          <div><h4>${t("usageModelMix")}</h4>${bars(total.models)}</div>
          <div>
            <div class="usage-heat-head"><h4>${t("usageHeatmap")}</h4>
              <div class="select"><select data-heat="${p}">
                <option value="">${t("usageHeatmapAll")}</option>
                ${accounts.filter((s) => s.tokens.total).map((s) => `<option value="${esc(s.id ?? "-")}" ${(s.id ?? "-") === pick ? "selected" : ""}>${esc(accountName(s))}</option>`).join("")}
              </select></div>
            </div>
            ${heatmap((heatFrom ?? total).heatmap)}
          </div>
        </div>` : ""}
        <div class="group">${accounts.length ? accounts.map((s) => usageAccount(s, slice(models, s))).join("") : `<div class="empty">${t("noAccounts")}</div>`}</div>
      </div>`;
  }).join("");
}

async function loadUsage() {
  renderUsage();
  try {
    usage = await Promise.all(["repo", "model"].map((group_by) => invoke("get_usage_breakdown", { days: usageDays, groupBy: group_by })));
  } catch { return; }
  renderUsage();
}

$("usage-days").innerHTML = USAGE_DAYS.map((n) => `<option value="${n}">${t("usageDays", { n })}</option>`).join("");
$("usage-days").value = String(usageDays);
$("usage-days").addEventListener("change", (e) => {
  usageDays = Number(e.target.value);
  try { localStorage.setItem("usage-days", String(usageDays)); } catch {}
  loadUsage();
});
$("usage").addEventListener("change", (e) => {
  const p = e.target.dataset.heat;
  if (p == null) return;
  heatPick[p] = e.target.value;
  renderUsage();
});
document.querySelector('.nav[data-page="usage"]').addEventListener("click", loadUsage);
listen("usage-updated", () => { if ($("page-usage").classList.contains("active")) loadUsage(); });

invoke("csv_default_folder").then((folder) => {
  let saved = "";
  try { saved = localStorage.getItem("export-folder") || ""; } catch {}
  $("export-folder").value = saved || folder || "";
}, () => {});
$("export-csv").addEventListener("click", async () => {
  const status = $("export-status");
  const folder = $("export-folder").value.trim();
  status.textContent = t("exportCsvSaving"); status.hidden = false;
  try {
    const written = await invoke("export_csv", { folder: folder || null });
    try { localStorage.setItem("export-folder", folder); } catch {}
    status.textContent = t("exportCsvSaved", { n: written.files.length, path: written.folder });
  } catch {
    status.textContent = t("exportCsvFailed");
  }
});

// ---------------------------------------------------------------------------------------------
// Backup and restore. Sign-ins are never in a backup; the passphrase is never stored.

const codeText = (prefix, code) => {
  const key = `${prefix}_${String(code ?? "")}`;
  const text = t(key);
  return text === key ? t(`${prefix}_failed`) : text;
};
let restoring = null;

function backupStatus(text) {
  const status = $("backup-status");
  status.textContent = text; status.hidden = !text;
}

function passphrase() {
  const value = $("backup-passphrase").value;
  if ([...value].length < 8) { backupStatus(t("backupErr_backup_passphrase_too_short")); return null; }
  return value;
}

$("backup-export").addEventListener("click", async () => {
  const pass = passphrase();
  if (!pass) return;
  backupStatus(t("backupExporting"));
  try {
    const written = await invoke("export_backup", { path: null, passphrase: pass });
    $("backup-passphrase").value = "";
    backupStatus(t("backupSaved", { path: written.path }));
  } catch (e) { backupStatus(codeText("backupErr", e)); }
});

$("backup-restore").addEventListener("click", () => {
  if (!passphrase()) return;
  $("backup-file").value = "";
  $("backup-file").click();
});

$("backup-file").addEventListener("change", async (e) => {
  const file = e.target.files?.[0];
  const pass = passphrase();
  if (!file || !pass) return;
  backupStatus(t("backupOpening"));
  try {
    const contents = await file.text();
    const preview = await invoke("restore_backup", { contents, passphrase: pass, dryRun: true });
    restoring = { contents, pass };
    renderBackupPreview(preview);
    backupStatus("");
  } catch (err) { restoring = null; $("backup-preview").hidden = true; backupStatus(codeText("backupErr", err)); }
});

function renderBackupPreview(p) {
  const list = (items) => items.length ? esc(items.join(", ")) : "—";
  const changes = (c) => t("backupChanges", { added: c.added.length, changed: c.changed.length });
  const account = (a) => `${esc(a.alias || a.email || a.id)} · ${a.connected ? t("backupConnected") : t("backupPending")}`;
  const rows = [
    [t("backupFrom"), esc(`${shortDate(p.created_at)} · ${p.app_version}`)],
    [t("backupPrefs"), list(p.settings)],
    [t("backupAliases"), changes(p.aliases)],
    [t("backupPlans"), changes(p.plans)],
    [t("backupPins"), changes(p.pins)],
    [t("backupHistory"), changes(p.history)],
    [t("backupLedger"), changes(p.ledger)],
    [t("backupAccounts"), p.accounts.map(account).join("<br>") || "—"],
  ];
  if (p.skipped.length) rows.push([t("backupSkipped"), esc(p.skipped.map((s) => s.what).join(", "))]);
  $("backup-preview").innerHTML = rows.map(([k, v]) => `<div class="row"><div class="title"><label>${esc(k)}</label><small>${v}</small></div></div>`).join("") +
    `<div class="row"><div class="title"><small>${esc(t("backupReconnect"))}</small></div>
      <div class="presence-path"><button class="btn accent" id="backup-apply">${esc(t("backupApply"))}</button><button class="btn" id="backup-cancel">${esc(t("backupCancel"))}</button></div></div>`;
  $("backup-preview").hidden = false;
  $("backup-cancel").addEventListener("click", () => { restoring = null; $("backup-preview").hidden = true; });
  $("backup-apply").addEventListener("click", async () => {
    if (!restoring) return;
    $("backup-apply").disabled = true;
    try {
      const done = await invoke("restore_backup", { contents: restoring.contents, passphrase: restoring.pass, dryRun: false });
      restoring = null;
      $("backup-passphrase").value = "";
      $("backup-preview").hidden = true;
      settings = await invoke("get_settings");
      await render();
      backupStatus(t("backupRestored", { n: done.pending }));
    } catch (err) { backupStatus(codeText("backupErr", err)); $("backup-apply").disabled = false; }
  });
}

// ---------------------------------------------------------------------------------------------
// Phone alerts (ntfy). The access token goes to its own DPAPI-protected file, never settings.

const PHONE_KINDS = ["renewal", "unexpected_use", "runaway", "limit_reached"];
const phoneStatus = (text) => { $("phone-status").textContent = text; $("phone-status").hidden = !text; };

async function renderPhone() {
  const phone = settings.phone;
  $("phone-enabled").checked = phone.enabled;
  $("phone-enabled").disabled = !phone.url;
  if (document.activeElement !== $("phone-url")) $("phone-url").value = phone.url ?? "";
  $("phone-include_names").checked = phone.include_names;
  for (const kind of PHONE_KINDS) $(`phone-kind-${kind}`).checked = phone.kinds[kind];
  $("phone-test").disabled = !phone.url;
  const saved = await invoke("phone_token_saved").catch(() => false);
  $("phone-token-state").textContent = saved ? t("phoneTokenSaved") : t("phoneTokenNone");
}

const updatePhone = (patch) => update({ phone: { ...settings.phone, ...patch } });
$("phone-enabled").addEventListener("change", (e) => updatePhone({ enabled: e.target.checked }));
$("phone-include_names").addEventListener("change", (e) => updatePhone({ include_names: e.target.checked }));
for (const kind of PHONE_KINDS) {
  $(`phone-kind-${kind}`).addEventListener("change", (e) => updatePhone({ kinds: { ...settings.phone.kinds, [kind]: e.target.checked } }));
}
$("phone-url-save").addEventListener("click", async () => {
  const url = $("phone-url").value.trim() || null;
  const before = settings;
  try {
    await update({ phone: { ...settings.phone, url } });
    settings = await invoke("get_settings");
    await render();
    phoneStatus(url ? t("phoneUrlSaved") : "");
  } catch {
    settings = before;
    await render();
    phoneStatus(t("phoneUrlInvalid"));
  }
});
$("phone-token-save").addEventListener("click", async () => {
  try {
    await invoke("set_phone_token", { token: $("phone-token").value.trim() || null });
    $("phone-token").value = "";
    phoneStatus("");
  } catch (e) { phoneStatus(codeText("phoneErr", e)); }
  renderPhone();
});
$("phone-token-clear").addEventListener("click", async () => {
  try { await invoke("set_phone_token", { token: null }); phoneStatus(""); } catch (e) { phoneStatus(codeText("phoneErr", e)); }
  renderPhone();
});
$("phone-test").addEventListener("click", async () => {
  phoneStatus(t("phoneTestSending"));
  try {
    await invoke("send_phone_test", { title: t("phoneTestTitle"), body: t("phoneTestBody") });
    phoneStatus(t("phoneTestSent"));
  } catch (e) { phoneStatus(codeText("phoneErr", e)); }
});

// ---------------------------------------------------------------------------------------------
// Account roster: the account list shared with the user's other computers, encrypted with a
// passphrase that is saved here with DPAPI and never shown again.

let rosterMessage = "";

async function renderRoster() {
  const placed = Boolean(settings.presence_folder || settings.presence_remote);
  $("account-roster").checked = settings.account_roster;
  $("account-roster").disabled = !placed && !settings.account_roster;
  const r = await invoke("get_roster").catch(() => null);
  $("roster-passphrase-state").textContent = r?.passphrase_saved ? t("rosterPassphraseSaved") : t("rosterPassphraseNone");
  $("roster-passphrase-clear").disabled = !r?.passphrase_saved;
  let text = rosterMessage;
  if (!text && settings.account_roster) {
    if (!placed) text = t("rosterNeedsSharing");
    else if (!r?.passphrase_saved) text = t("rosterNeedsPassphrase");
    else if (r.wrote === false) text = t("rosterWriteFailed");
    else if (r.read === false) text = t("rosterReadFailed");
    else if (r.read) {
      text = [t("rosterShared", { n: r.machines.length }), r.unreadable ? t("rosterUnreadable", { n: r.unreadable }) : "",
        r.waiting.length ? `${t("rosterWaiting", { n: r.waiting.length })}.` : ""].filter(Boolean).join(" ");
    }
  }
  $("roster-status").textContent = text;
  $("roster-status").hidden = !text;
}

$("account-roster").addEventListener("change", async (e) => {
  rosterMessage = "";
  await update({ account_roster: e.target.checked });
  renderRoster();
  loadBulk();
});
async function saveRosterPassphrase(passphrase) {
  rosterMessage = "";
  if (passphrase !== null && [...passphrase].length < 8) { rosterMessage = t("rosterErr_roster_passphrase_too_short"); return renderRoster(); }
  $("roster-passphrase-save").disabled = true;
  try {
    await invoke("save_roster_passphrase", { passphrase });
    $("roster-passphrase").value = "";
  } catch (e) { rosterMessage = codeText("rosterErr", e); }
  $("roster-passphrase-save").disabled = false;
  await renderRoster();
  loadBulk();
}
$("roster-passphrase-save").addEventListener("click", () => saveRosterPassphrase($("roster-passphrase").value));
$("roster-passphrase").addEventListener("keydown", (e) => { if (e.key === "Enter") saveRosterPassphrase($("roster-passphrase").value); });
$("roster-passphrase-clear").addEventListener("click", () => saveRosterPassphrase(null));
listen("usage-updated", () => { if (settings) renderRoster(); });

// ---------------------------------------------------------------------------------------------
// Everything else

async function render() {
  setLanguage(settings.language);
  translateDom();
  document.title = `SlopBar · ${t("settings")}`;

  $("refresh").innerHTML = REFRESH_OPTIONS.map((n) => `<option value="${n}">${t("minutes", { n })}</option>`).join("");
  $("refresh").value = String(settings.refresh_minutes);

  $("language").innerHTML = `<option value="auto">${t("langAuto")}</option>` +
    Object.entries(LANGUAGES).map(([code, name]) => `<option value="${code}">${name}</option>`).join("");
  $("language").value = settings.language;

  $("claude").checked = settings.providers.claude;
  $("codex").checked = settings.providers.codex;
  $("wsl-accounts").innerHTML = ["auto", "on", "off"].map((v) => `<option value="${v}">${t(`wsl_${v}`)}</option>`).join("");
  $("wsl-accounts").value = settings.wsl_accounts ?? "auto";
  $("signin-window").innerHTML = ["profile", "private", "system"].map((v) => `<option value="${v}">${t(`signinWindow_${v}`)}</option>`).join("");
  $("signin-window").value = settings.signin_window ?? "profile";
  $("startup").checked = settings.launch_at_login;
  $("auto-update").checked = updatesEnabled && settings.auto_update;
  $("auto-update").disabled = !updatesEnabled;
  for (const kind of NOTIFY_KINDS) $(`notify-${kind}`).checked = settings.notifications[kind];
  $("crash-records").checked = settings.crash_records;
  $("auto-monitoring").checked = settings.auto_connect_monitoring;
  $("compact-flyout").checked = settings.compact_flyout;
  $("compact-flyout-hint").textContent = snapshot.accounts.length > COMPACT_SUGGEST_ABOVE && !settings.compact_flyout
    ? t("compactFlyoutSuggested", { n: snapshot.accounts.length }) : t("compactFlyoutHint");
  document.querySelectorAll("input.switch").forEach((el) => el.setAttribute("role", "switch"));
  renderGuide();
  renderCalendar();
  renderPhone();
  renderPresence();
  renderRoster();
  const version = await invoke("app_version");
  $("version").textContent = /^\d/.test(version) ? `v${version}` : version;
  $("version-label").textContent = t("versionN", { v: version });
  if (!updatesEnabled) {
    $("check-updates").disabled = true;
    $("check-updates").textContent = t("checkUpdates");
    $("update-status").textContent = t("manualUpdates");
  } else if (!$("check-updates").disabled) $("check-updates").textContent = latest ? t("installUpdate") : t("checkUpdates");
  renderAccounts();
  if (!editingPlan()) renderSubscriptions();
  renderHotkey();
  loadBulk();
  loadPins();
}

async function update(patch) {
  settings = { ...settings, ...patch };
  await render();
  await invoke("save_settings", { settings });
}

$("language").addEventListener("change", (e) => update({ language: e.target.value }));
$("refresh").addEventListener("change", (e) => update({ refresh_minutes: Number(e.target.value) }));
$("claude").addEventListener("change", (e) => update({ providers: { ...settings.providers, claude: e.target.checked } }));
$("codex").addEventListener("change", (e) => update({ providers: { ...settings.providers, codex: e.target.checked } }));
$("startup").addEventListener("change", (e) => update({ launch_at_login: e.target.checked }));
$("auto-update").addEventListener("change", (e) => update({ auto_update: e.target.checked }));
$("export-diagnostics").addEventListener("click", async () => {
  const button = $("export-diagnostics"), status = $("diagnostics-status");
  button.disabled = true; status.hidden = false; status.textContent = t("diagnosticsExporting");
  try {
    const path = await invoke("export_diagnostics");
    status.textContent = t("diagnosticsSaved", { path });
  } catch { status.textContent = t("diagnosticsFailed"); }
  finally { button.disabled = false; }
});
$("crash-records").addEventListener("change", (e) => update({ crash_records: e.target.checked }));
$("auto-monitoring").addEventListener("change", (e) => update({ auto_connect_monitoring: e.target.checked }));
$("wsl-accounts").addEventListener("change", (e) => update({ wsl_accounts: e.target.value }));
$("signin-window").addEventListener("change", (e) => update({ signin_window: e.target.value }));
// Reads every WSL distro once, starting stopped ones, then refreshes.
$("wsl-rescan").addEventListener("click", async () => {
  const button = $("wsl-rescan");
  button.disabled = true;
  $("wsl-rescan-hint").textContent = t("wslRescanning");
  try { snapshot = await invoke("rescan_wsl"); renderAccounts(); await loadHealth(); }
  finally { button.disabled = false; $("wsl-rescan-hint").textContent = t("wslRescanHint"); }
});
$("compact-flyout").addEventListener("change", (e) => update({ compact_flyout: e.target.checked, compact_suggested: true }));

// ---------------------------------------------------------------------------------------------
// The first-run guide (Accounts): shown while no account is found or none is monitored, until
// closed; General opens it again. Every sign-in it offers is the existing flow the user finishes.

const COMPACT_SUGGEST_ABOVE = 8;
const GUIDE_NOTIFY = ["usage", "renewals", "unexpected_use"];
const NOTIFY_LABEL = { usage: "notifyUsage", renewals: "notifyRenewals", unexpected_use: "notifyUnexpected" };
let guideForced = false;

const guideShown = () => guideForced || (!settings.onboarding_dismissed
  && (!snapshot.accounts.length || !snapshot.accounts.some((a) => a.monitor_connected)));

function guideStep(n, title, state, body) {
  const label = t({ done: "guideStepDone", todo: "guideStepTodo", optional: "guideStepOptional" }[state]);
  return `
    <li class="guide-step ${state}">
      <div class="guide-step-head"><span class="guide-num" aria-hidden="true">${state === "done" ? "✓" : n}</span><h3>${esc(title)}</h3><em class="chip ${state === "done" ? "elsewhere" : ""}">${esc(label)}</em></div>
      <div class="guide-body">${body}</div>
    </li>`;
}

function renderGuide() {
  const box = $("guide");
  const show = guideShown();
  box.hidden = !show;
  // Not under an open dropdown; elsewhere the focused control keeps focus across the rebuild.
  const el = box.contains(document.activeElement) ? document.activeElement : null;
  if (!show || el?.tagName === "SELECT") return;
  const focused = !el ? null : el.id ? `#${CSS.escape(el.id)}`
    : el.dataset.guideNotify ? `[data-guide-notify="${el.dataset.guideNotify}"]`
    : el.dataset.guideMonitor ? `[data-guide-monitor][data-id="${CSS.escape(el.dataset.id)}"]` : null;
  const list = snapshot.accounts.filter((a) => settings.providers[a.provider]);
  const count = (p) => list.filter((a) => a.provider === p).length;
  const monitored = list.filter((a) => a.monitor_connected);
  const waiting = list.filter((a) => !a.monitor_connected);
  const name = (a) => a.alias || a.email || t("unknownAccount");
  const presence = presenceChoices();
  box.innerHTML = `
    <div class="guide-head">
      <h2 id="guide-title" tabindex="-1">${t("guideTitle")}</h2>
      <button class="remove guide-close" id="guide-close" title="${esc(t("guideClose"))}" aria-label="${esc(t("guideClose"))}"><svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8"/></svg></button>
    </div>
    <p class="hint guide-intro">${t("guideIntro")}</p>
    <ol class="guide-steps">
      ${guideStep(1, t("guideFindTitle"), list.length ? "done" : "todo",
        `<p>${esc(list.length ? t("guideFound", { claude: count("claude"), codex: count("codex") }) : t("guideFoundNone"))}</p>`)}
      ${guideStep(2, t("guideMonitorTitle"), list.length && !waiting.length ? "done" : "todo", `
        <p>${t("guideMonitorBody")} ${esc(list.length ? (waiting.length ? t("guideMonitorCount", { n: monitored.length, total: list.length }) : t("guideMonitorAll")) : "")}</p>
        ${waiting.length ? `<ul class="guide-accounts">${waiting.map((a) => `
          <li><span class="logo ${a.provider}" role="img" aria-label="${a.provider === "claude" ? "Claude" : "Codex"}">${MARK[a.provider]}</span><span class="guide-name">${esc(name(a))}</span>
            <button class="btn" data-guide-monitor="${a.provider}" data-id="${esc(a.id)}" aria-label="${esc(t("onAccount", { action: t("monitorConnect"), name: name(a) }))}">${t("monitorConnect")}</button></li>`).join("")}</ul>` : ""}
        <p class="hint">${t(settings.auto_connect_monitoring ? "guideAutoOn" : "guideAutoOff")}</p>`)}
      ${guideStep(3, t("guideShareTitle"), settings.presence_folder || settings.presence_remote ? "done" : "optional", `
        <p>${t("guideShareBody")}</p>
        <div class="guide-share">
          <label for="guide-presence">${t("guideShareLabel")}</label>
          <div class="select"><select id="guide-presence">${presence.html}</select></div>
          <button class="btn" id="guide-presence-other">${t("guideShareOther")}</button>
        </div>`)}
      ${guideStep(4, t("guideNotifyTitle"), GUIDE_NOTIFY.every((k) => settings.notifications[k]) ? "done" : "todo", `
        <p>${t("guideNotifyBody")}</p>
        <div class="guide-notify">${GUIDE_NOTIFY.map((k) => `
          <label><input type="checkbox" class="switch" role="switch" data-guide-notify="${k}" ${settings.notifications[k] ? "checked" : ""} /> ${t(NOTIFY_LABEL[k])}</label>`).join("")}</div>`)}
    </ol>
    <div class="guide-foot"><button class="btn accent" id="guide-done">${t("guideDone")}</button></div>
    <p class="hint" id="guide-status" role="status" hidden></p>`;
  $("guide-presence").value = presence.known ? presence.value : "";
  if (focused) box.querySelector(focused)?.focus();
}

async function closeGuide() {
  guideForced = false;
  await update({ onboarding_dismissed: true });
  // Focus stays on the page the guide was on.
  const heading = document.querySelector("#page-accounts h1");
  heading.setAttribute("tabindex", "-1");
  heading.focus();
}
$("guide").addEventListener("click", async (e) => {
  if (e.target.closest("#guide-close, #guide-done")) { closeGuide(); return; }
  if (e.target.closest("#guide-presence-other")) {
    showPage("data");
    $("presence").value = "__other__";
    $("presence").dispatchEvent(new Event("change"));
    return;
  }
  const monitor = e.target.closest("[data-guide-monitor]");
  if (!monitor) return;
  monitor.disabled = true;
  const status = $("guide-status");
  try {
    await invoke("connect_monitoring", { provider: monitor.dataset.guideMonitor, id: monitor.dataset.id });
    status.textContent = t("monitorPending");
  } catch (err) { status.textContent = errorText("", String(err)); monitor.disabled = false; }
  status.hidden = false;
});
$("guide").addEventListener("change", (e) => {
  const el = e.target;
  if (el.id === "guide-presence") choosePresence(el.value);
  else if (el.dataset.guideNotify) update({ notifications: { ...settings.notifications, [el.dataset.guideNotify]: el.checked } });
});
$("guide-reopen").addEventListener("click", async () => {
  guideForced = true;
  showPage("accounts");
  await update({ onboarding_dismissed: false });
  $("guide-title")?.focus();
  $("guide").scrollIntoView({ block: "start" });
});

// The plan-renewal calendar (calendar.rs): an .ics file calendar apps can subscribe to.
let calendarDefault = "";
invoke("default_calendar_path").then((p) => { calendarDefault = p ?? ""; if (settings) renderCalendar(); }, () => {});
function renderCalendar() {
  $("calendar-enabled").checked = settings.calendar?.enabled ?? false;
  if (document.activeElement !== $("calendar-path")) $("calendar-path").value = settings.calendar?.path ?? calendarDefault;
}
const calendarStatus = (text) => { $("calendar-status").textContent = text; $("calendar-status").hidden = !text; };
async function exportCalendar() {
  try {
    const { path, events } = await invoke("export_calendar");
    calendarStatus(t("calendarSaved", { n: events, path }));
  } catch { calendarStatus(t("calendarFailed")); }
}
$("calendar-enabled").addEventListener("change", async (e) => {
  await update({ calendar: { ...settings.calendar, enabled: e.target.checked } });
  if (e.target.checked) exportCalendar();
});
$("calendar-save").addEventListener("click", async () => {
  const path = $("calendar-path").value.trim();
  if (path && path !== calendarDefault && !(await invoke("check_calendar_path", { path }))) { calendarStatus(t("calendarPathInvalid")); return; }
  calendarStatus("");
  await update({ calendar: { ...settings.calendar, path: path && path !== calendarDefault ? path : null } });
  if (settings.calendar.enabled) exportCalendar();
});
$("calendar-export").addEventListener("click", exportCalendar);

// ---------------------------------------------------------------------------------------------
// Health: the backend's self-check (health.rs) with each check's fix wired to the matching action.

let health = null;
const FIX_PAGES = { data: "sData", notifications: "sNotifications", general: "sGeneral" };

function fixButton(fix, i) {
  if (!fix) return "";
  const label = {
    connect_monitoring: () => t("monitorConnect"),
    refresh: () => t("healthFixRefresh"),
    restart_app: () => t("healthFixRestart"),
    open_settings: () => `${t("healthFixOpen")} ${t(FIX_PAGES[fix.page] ?? "settings")}`,
  }[fix.action]?.();
  return label ? `<button class="btn" data-fix="${i}">${esc(label)}</button>` : "";
}

function renderHealth() {
  if (!health) return;
  $("health").innerHTML = health.checks.map((c, i) => `
    <div class="row health-check" data-key="${esc(c.id)}">
      <div class="title"><label><span class="health-dot ${c.status}" role="img" aria-label="${esc(t({ ok: "healthOk", warning: "healthWarning", problem: "healthProblem" }[c.status] ?? "healthOk"))}"></span>${esc(c.title)}</label><small>${esc(c.reason)}</small></div>
      ${c.status === "ok" ? "" : fixButton(c.fix, i)}
    </div>`).join("");
  const badge = $("health-nav-badge");
  badge.hidden = !health.problems;
  badge.textContent = String(health.problems);
}

async function loadHealth() {
  try { health = await invoke("get_health"); } catch { return; }
  renderHealth();
}

$("health").addEventListener("click", async (e) => {
  const button = e.target.closest("[data-fix]");
  const fix = button && health?.checks[Number(button.dataset.fix)]?.fix;
  if (!fix) return;
  const status = $("health-status");
  button.disabled = true;
  try {
    if (fix.action === "connect_monitoring") {
      await invoke("connect_monitoring", { provider: fix.provider, id: fix.id });
      status.textContent = t("monitorPending"); status.hidden = false;
    } else if (fix.action === "refresh") {
      snapshot = await invoke("refresh");
      renderAccounts();
    } else if (fix.action === "restart_app") {
      status.textContent = t("healthRestarting"); status.hidden = false;
      await invoke("restart_app");
    } else if (fix.action === "open_settings") {
      document.querySelector(`.nav[data-page="${fix.page}"]`)?.click();
    }
  } catch (err) { status.textContent = errorText("", String(err)); status.hidden = false; }
  finally { button.disabled = false; }
  loadHealth();
});
listen("usage-updated", loadHealth);
listen("monitoring-login", loadHealth);
document.querySelector('.nav[data-page="health"]').addEventListener("click", loadHealth);

// Sharing in-use status: off, a detected sync folder (OneDrive, Dropbox, Google Drive…), an rclone
// remote (any cloud, no sync app needed) or a typed folder.
let syncFolders = [];
let rclone = { installed: true, remotes: [] };
let presenceOther = false;
const REMOTE = "rclone:";
/** The sharing choices: off, detected sync folders and rclone remotes (as list_sharing_locations). */
function presenceChoices() {
  const remote = settings.presence_remote ?? "";
  const value = remote ? REMOTE + remote : settings.presence_folder ?? "";
  const remotes = rclone.remotes.some((r) => r.name === remote) || !remote ? rclone.remotes : [...rclone.remotes, { name: remote, provider: "rclone" }];
  const known = remote !== "" || value === "" || syncFolders.some((f) => f.path === value);
  const html = `<option value="">${t("presenceOff")}</option>`
    + syncFolders.map((f) => `<option value="${esc(f.path)}">${esc(f.name)} · ${esc(f.path)}</option>`).join("")
    + remotes.map((r) => `<option value="${esc(REMOTE + r.name)}">${esc(t("presenceViaRclone", { provider: r.provider }))} · ${esc(r.name)}:</option>`).join("");
  return { html, value, known };
}
function choosePresence(value) {
  if (value.startsWith(REMOTE)) return update({ presence_remote: value.slice(REMOTE.length), presence_folder: null });
  return update({ presence_folder: value || null, presence_remote: null });
}
function renderPresence() {
  const { html, value: current, known } = presenceChoices();
  const other = presenceOther || !known;
  $("presence").innerHTML = html + `<option value="__other__">${t("presenceOther")}</option>`;
  $("presence").value = other ? "__other__" : current;
  $("presence-custom").hidden = !other;
  $("presence-path").placeholder = t("presenceFolderPlaceholder");
  if (other && !known && document.activeElement !== $("presence-path")) $("presence-path").value = current;
  $("presence-rclone-hint").hidden = rclone.installed;
}
$("presence").addEventListener("change", (e) => {
  $("presence-status").hidden = true;
  const value = e.target.value;
  presenceOther = value === "__other__";
  if (presenceOther) { renderPresence(); $("presence-path").focus(); return; }
  choosePresence(value);
});
async function savePresencePath() {
  const folder = $("presence-path").value.trim();
  if (!folder) return;
  if (!(await invoke("check_presence_folder", { folder }))) {
    $("presence-status").textContent = t("presenceFolderInvalid");
    $("presence-status").hidden = false;
    return;
  }
  $("presence-status").hidden = true;
  update({ presence_folder: folder, presence_remote: null });
}
$("presence-save").addEventListener("click", savePresencePath);
$("presence-path").addEventListener("keydown", (e) => { if (e.key === "Enter") savePresencePath(); });
invoke("sync_folders").then((folders) => { syncFolders = folders ?? []; if (settings) { renderPresence(); renderGuide(); } }, () => {});
invoke("rclone_remotes").then((found) => { rclone = found ?? rclone; if (settings) { renderPresence(); renderGuide(); } }, () => {});

for (const kind of NOTIFY_KINDS) {
  $(`notify-${kind}`).addEventListener("change", (e) => update({ notifications: { ...settings.notifications, [kind]: e.target.checked } }));
}
document.querySelectorAll("[data-mark]").forEach((el) => (el.innerHTML = MARK[el.dataset.mark]));

// Updates: checking shows the result inline; when one is available the button installs it.
let latest = null;
function showUpdateState(state) {
  const status = $("update-status");
  const button = $("check-updates");
  status.classList.toggle("available", state === "available");
  status.textContent = state === "checking" ? t("checking")
    : state === "available" ? t("updateAvailable", { v: latest })
    : state === "current" ? t("upToDate")
    : state === "installing" ? t("updating", { v: latest })
    : state ? String(state) : "";
  button.textContent = state === "available" ? t("installUpdate") : t("checkUpdates");
  button.classList.toggle("accent", state === "available");
  button.disabled = state === "checking" || state === "installing";
}
$("check-updates").addEventListener("click", async () => {
  if (latest) {
    showUpdateState("installing");
    try { await invoke("install_update"); } catch (err) { showUpdateState(String(err)); }
    return;
  }
  showUpdateState("checking");
  try {
    const info = await invoke("check_for_updates");
    latest = info.latest;
    showUpdateState(latest ? "available" : "current");
  } catch (err) {
    showUpdateState(String(err));
  }
});

listen("settings-changed", (e) => { settings = e.payload; render(); });

// ---------------------------------------------------------------------------------------------
// The quick switcher's shortcut (General)

async function renderHotkey() {
  if (document.activeElement !== $("hotkey")) $("hotkey").value = settings.switcher_hotkey ?? "";
  $("hotkey").placeholder = t("hotkeyOff");
  const problem = settings.switcher_hotkey ? await invoke("hotkey_status").catch(() => null) : null;
  $("hotkey-hint").textContent = problem ? t(problem) : settings.switcher_hotkey ? t("switcherShortcutHint") : t("hotkeyOffHint");
  $("hotkey-hint").classList.toggle("problem", !!problem);
}
async function saveHotkey() {
  const before = settings.switcher_hotkey;
  settings = { ...settings, switcher_hotkey: $("hotkey").value.trim() };
  try {
    await invoke("save_settings", { settings });
    settings = await invoke("get_settings");
    $("hotkey").value = settings.switcher_hotkey;
  } catch {
    settings = { ...settings, switcher_hotkey: before };
    $("hotkey-hint").textContent = t("hotkey_invalid");
    $("hotkey-hint").classList.add("problem");
    return;
  }
  renderHotkey();
}
$("hotkey-save").addEventListener("click", saveHotkey);
$("hotkey").addEventListener("keydown", (e) => { if (e.key === "Enter") saveHotkey(); });

// ---------------------------------------------------------------------------------------------
// Bulk actions (Accounts): monitoring for every unmonitored account, one sign-in at a time, and
// removing accounts whose plan has ended.

let bulk = { unmonitored: [], lapsed: { remove: [], in_use: [] }, connecting: null, sign_in: [], signin_profiles: 0 };
const who = (target) => target.account || t("unknownAccount");

/** "Sign in all accounts" as it would run now: its groups (one per email) and sign-ins. */
function signInPlan() {
  const monitoring = $("sign-in-all-monitoring").checked;
  const groups = (bulk.sign_in ?? [])
    .map((g) => ({ ...g, steps: g.steps.filter((s) => monitoring || s.kind === "add") }))
    .filter((g) => g.steps.length);
  return { groups, steps: groups.reduce((n, g) => n + g.steps.length, 0), monitoring };
}

function renderBulk() {
  const c = bulk.connecting;
  const running = c && !c.finished;
  const plan = signInPlan();
  // While one runs, either button stops it (its sign-in closes with its browser).
  $("sign-in-all").disabled = !running && !plan.steps;
  $("sign-in-all").textContent = t(running ? "signInAllStop" : "signInAllButton");
  $("bulk-connect").textContent = t(running ? "signInAllStop" : "bulkConnectButton");
  $("sign-in-all-hint").textContent = plan.steps ? t("signInAllHint", { n: plan.steps, e: plan.groups.length }) : t("signInAllNone");
  $("signin-forget").disabled = !bulk.signin_profiles;
  $("signin-forget-hint").textContent = bulk.signin_profiles ? t("signinForgetHint", { n: bulk.signin_profiles }) : t("signinForgetNone");
  $("bulk-connect").disabled = !running && !bulk.unmonitored.length;
  $("bulk-connect-hint").textContent = bulk.unmonitored.length ? t("bulkConnectHint", { n: bulk.unmonitored.length }) : t("bulkConnectNone");
  $("bulk-remove").disabled = !bulk.lapsed.remove.length;
  $("bulk-remove-hint").textContent = bulk.lapsed.remove.length
    ? t("bulkRemoveHint", { list: bulk.lapsed.remove.map(who).join(", ") })
    : bulk.lapsed.in_use.length ? t("bulkRemoveInUse", { list: bulk.lapsed.in_use.map(who).join(", ") }) : t("bulkRemoveNone");
  const status = $("bulk-status");
  const where = settings.signin_window === "system" ? t("signInWhereTerminal") : t("signInWhereBrowser");
  if (running && c.current && c.still_waiting) {
    status.textContent = t("signInStillWaiting", { account: who(c.current), where });
  } else if (running && c.current && c.emails) {
    status.textContent = t(c.kind === "add" ? "signInAllAdding" : "signInAllConnecting",
      { g: c.email, e: c.emails, i: c.done + 1, n: c.total, account: who(c.current), where });
  } else if (running && c.current) status.textContent = t("bulkConnecting", { i: c.done + 1, n: c.total, account: who(c.current) });
  else if (c?.finished && c.error) status.textContent = t("bulkConnectStopped", { done: c.done, n: c.total, error: errorText("", c.error) });
  else if (c?.finished && c.total) status.textContent = t(c.emails ? "signInAllDone" : "bulkConnectDone", { n: c.total });
  status.hidden = !(running || (c?.finished && c.total)) || !status.textContent;
  // A stopped or failed run, or a sign-in left waiting: Retry runs it again with what is still missing.
  $("bulk-retry-row").hidden = !((c?.finished && c.error) || (running && c.still_waiting));
}
async function loadBulk() {
  bulk = await invoke("bulk_status").catch(() => bulk);
  renderBulk();
}
const stopping = () => bulk.connecting && !bulk.connecting.finished;
$("sign-in-all").addEventListener("click", async () => {
  if (stopping()) return void invoke("stop_sign_ins").finally(loadBulk);
  $("sign-in-all").disabled = true;
  try { await invoke("sign_in_all_accounts", { monitoring: $("sign-in-all-monitoring").checked }); } catch (err) { alert(errorText("", String(err))); }
  loadBulk();
});
$("sign-in-all-monitoring").addEventListener("change", renderBulk);
$("bulk-retry").addEventListener("click", async () => {
  $("bulk-retry").disabled = true;
  try { await invoke("retry_sign_ins"); } catch (err) { alert(errorText("", String(err))); }
  finally { $("bulk-retry").disabled = false; }
  loadBulk();
});
$("signin-forget").addEventListener("click", async () => {
  if (!confirm(t("signinForgetConfirm", { n: bulk.signin_profiles }))) return;
  $("signin-forget").disabled = true;
  const status = $("bulk-status");
  try {
    status.textContent = t("signinForgotten", { n: await invoke("forget_signin_profiles") });
  } catch (err) {
    status.textContent = errorText("", String(err));
  }
  status.hidden = false;
  loadBulk();
});
$("bulk-connect").addEventListener("click", async () => {
  if (stopping()) return void invoke("stop_sign_ins").finally(loadBulk);
  $("bulk-connect").disabled = true;
  try { await invoke("connect_monitoring_all"); } catch (err) { alert(errorText("", String(err))); }
  loadBulk();
});
$("bulk-remove").addEventListener("click", async () => {
  await loadBulk();
  const list = bulk.lapsed.remove;
  if (!list.length) return;
  const lines = list.map((a) => `• ${who(a)} (${a.provider === "claude" ? "Claude" : "Codex"}) — ${t("bulkEnded", { d: shortDay(a.ended) })}`);
  if (!confirm(`${t("bulkRemoveConfirm", { n: list.length })}\n\n${lines.join("\n")}`)) return;
  $("bulk-remove").disabled = true;
  const result = await invoke("remove_lapsed_accounts").catch((err) => ({ removed: [], failed: [{ error: String(err) }] }));
  if (result.snapshot) { snapshot = result.snapshot; renderAccounts(); }
  const status = $("bulk-status");
  status.textContent = result.failed.length
    ? t("bulkRemoveFailed", { n: result.removed.length, failed: result.failed.length })
    : t("bulkRemoved", { n: result.removed.length });
  status.hidden = false;
  loadBulk();
});
listen("bulk-monitoring", ({ payload }) => { bulk.connecting = payload; renderBulk(); if (payload.finished) loadBulk(); });
listen("usage-updated", loadBulk);

// ---------------------------------------------------------------------------------------------
// "Signing in on <machine>" (Accounts): another computer's Sign in all accounts, as its roster
// carries it, followed while it goes on, with Retry over SSH (see `peer_sign_ins`).

let peerRuns = [];
/** What the last Retry on each machine's card said, by machine id. */
const peerNotes = {};

/** A card's lines: where the run is, and what the person here should know about it. */
function peerRunLines(p) {
  const r = p.run;
  const machine = p.machine;
  const current = r.steps.find((s) => s.state === "running");
  if (r.finished && r.error) {
    return [t("bulkConnectStopped", { done: r.done, n: r.total, error: errorText("", r.error) }), t("peerSignInStoppedHint", { machine })];
  }
  if (r.finished) return [t(r.total ? "signInAllDone" : "signInAllNone", { n: r.total }), ""];
  const step = !current ? t("peerSignInStarting")
    : r.still_waiting ? t("peerSignInWaiting", { machine, account: who(current) })
    : t(current.kind === "add" ? "signInAllAdding" : "signInAllConnecting",
      { g: r.email, e: r.emails, i: r.done + 1, n: r.total, account: who(current), where: t("signInWhereMachine", { machine }) });
  const quiet = Math.max(1, Math.round((Date.now() - new Date(p.written_at)) / 60000));
  return [step, p.quiet ? t("peerSignInQuiet", { machine, t: duration(quiet) }) : t("peerSignInCoach", { machine })];
}

function renderPeerRuns() {
  const box = $("peer-sign-ins");
  box.replaceChildren(...peerRuns.map((p) => {
    // Names and accounts come from other machines: text only, never markup.
    const card = document.createElement("div");
    card.className = "hint notice peer-run";
    card.setAttribute("role", "status");
    const title = document.createElement("h3");
    title.textContent = t("peerSigningIn", { machine: p.machine });
    card.append(title);
    for (const [text, dim] of peerRunLines(p).map((line, i) => [line, i > 0])) {
      if (!text) continue;
      const line = document.createElement("p");
      line.textContent = text;
      if (dim) line.className = "dim";
      card.append(line);
    }
    const r = p.run;
    if ((r.finished && r.error) || r.still_waiting || p.quiet) {
      if (p.reachable) {
        const retry = document.createElement("button");
        retry.className = "btn";
        retry.textContent = t("signInRetry");
        retry.addEventListener("click", () => retryPeer(p, retry));
        card.append(retry);
      } else {
        peerNotes[p.machine_id] ??= t("peerSignInNotPaired", { machine: p.machine });
      }
    }
    if (peerNotes[p.machine_id]) {
      const note = document.createElement("p");
      note.className = "dim";
      note.textContent = peerNotes[p.machine_id];
      card.append(note);
    }
    return card;
  }));
}

async function retryPeer(p, button) {
  button.disabled = true;
  peerNotes[p.machine_id] = t("peerSignInRetrying", { machine: p.machine });
  renderPeerRuns();
  try {
    await invoke("retry_peer_sign_in", { machineId: p.machine_id });
    peerNotes[p.machine_id] = t("peerSignInRetrySent", { machine: p.machine });
    // The new run reaches the roster seconds later: followed closely meanwhile.
    peerExpectUntil = Date.now() + 120000;
  } catch (err) {
    const code = String(err);
    peerNotes[p.machine_id] = code === "peer_not_paired" ? t("peerSignInNotPaired", { machine: p.machine })
      : t("peerSignInRetryFailed", { machine: p.machine, error: errorText("", code) });
  }
  loadPeerRuns();
}

let peerTimer = null;
let peerExpectUntil = 0;
async function loadPeerRuns() {
  clearTimeout(peerTimer);
  // Followed closely while Accounts is in view (and one goes on): the rosters are read sooner.
  const watching = !document.hidden && $("page-accounts").classList.contains("active");
  peerRuns = await invoke("peer_sign_ins", { watching }).catch(() => peerRuns);
  renderPeerRuns();
  const live = peerRuns.some((p) => !p.run.finished) || Date.now() < peerExpectUntil;
  peerTimer = setTimeout(loadPeerRuns, watching && live ? 15000 : 60000);
}
document.addEventListener("visibilitychange", () => { if (!document.hidden) loadPeerRuns(); });
document.querySelectorAll(".nav").forEach((b) => b.addEventListener("click", () => { if (b.dataset.page === "accounts") loadPeerRuns(); }));
const firstPeerRuns = loadPeerRuns();

// ---------------------------------------------------------------------------------------------
// An account per project (Accounts): pinned folders, and the lines that use them.

let pinListing = { pins: [], launcher: null };
const PIN_STATE = { ready: "pinReady", at_default: "pinAtDefault", no_login: "pinNoLogin" };

function renderPins() {
  $("pins").innerHTML = pinListing.pins.length ? pinListing.pins.map((p) => `
    <div class="row pin" data-pin="${esc(p.provider)}|${esc(p.folder)}">
      <span class="logo ${p.provider}">${MARK[p.provider]}</span>
      <div class="title">
        <label title="${esc(p.folder)}">${esc(p.folder)}</label>
        <small>${esc(p.account ?? t("unknownAccount"))} · ${esc(t(PIN_STATE[p.state]))}</small>
      </div>
      <button class="btn" data-copy="${esc(p.launch)}" title="${esc(p.launch)}">${t("pinCopyLaunch")}</button>
      <button class="btn" data-copy="${esc(p.powershell)}" title="${esc(p.powershell)}">${t("pinCopyPowerShell")}</button>
      <button class="btn" data-copy="${esc(p.envrc)}" title="${esc(p.envrc)}">${t("pinCopyEnvrc")}</button>
      <button class="remove" data-unpin="${esc(p.provider)}" data-folder="${esc(p.folder)}" title="${t("pinRemove")}">
        <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8"/></svg>
      </button>
    </div>`).join("") : `<div class="empty">${t("pinsNone")}</div>`;
  const choice = $("pin-account").value;
  $("pin-account").innerHTML = snapshot.accounts.map((a) =>
    `<option value="${esc(a.provider)}|${esc(a.id)}">${a.provider === "claude" ? "Claude" : "Codex"} · ${esc(a.alias || a.email || t("unknownAccount"))}</option>`).join("");
  if (choice) $("pin-account").value = choice;
  $("pin-folder").placeholder = t("pinFolderPlaceholder");
  $("pin-save").disabled = !snapshot.accounts.length;
  $("pins-launcher").textContent = pinListing.launcher ? t("pinsLauncher", { path: pinListing.launcher }) : "";
}
async function loadPins() {
  pinListing = await invoke("list_project_pins").catch(() => pinListing);
  renderPins();
}
function pinStatus(text) {
  $("pins-status").textContent = text;
  $("pins-status").hidden = !text;
}
async function savePin() {
  const folder = $("pin-folder").value.trim();
  const [provider, id] = $("pin-account").value.split(/\|(.*)/s);
  if (!folder || !provider) return;
  try {
    pinListing = await invoke("set_project_pin", { folder, provider, id });
    $("pin-folder").value = "";
    pinStatus("");
    renderPins();
  } catch (err) { pinStatus(String(err).includes("folder") ? t("presenceFolderInvalid") : String(err)); }
}
$("pin-save").addEventListener("click", savePin);
$("pin-folder").addEventListener("keydown", (e) => { if (e.key === "Enter") savePin(); });
$("pins").addEventListener("click", async (e) => {
  const copy = e.target.closest("[data-copy]");
  if (copy) {
    try { await navigator.clipboard.writeText(copy.dataset.copy); pinStatus(t("pinCopied")); } catch { pinStatus(copy.dataset.copy); }
    return;
  }
  const unpin = e.target.closest("[data-unpin]");
  if (unpin) {
    try {
      pinListing = await invoke("set_project_pin", { folder: unpin.dataset.folder, provider: unpin.dataset.unpin, id: null });
      renderPins();
    } catch (err) { pinStatus(String(err)); }
  }
});
listen("usage-updated", loadPins);

// The window draws its own title bar (see settings.html).
$("close").addEventListener("click", () => window.__TAURI__.window.getCurrentWindow().close());
window.addEventListener("blur", () => document.body.classList.add("inactive"));
window.addEventListener("focus", () => document.body.classList.remove("inactive"));
invoke("has_mica").then((mica) => document.documentElement.classList.toggle("mica", mica), () => {});

/** The system accent color, as WinUI tints it for light and dark backgrounds. */
async function applyAccent() {
  const colors = await invoke("accent_colors").catch(() => null);
  if (!colors) return;
  document.documentElement.style.setProperty("--accent-light", colors[0]);
  document.documentElement.style.setProperty("--accent-dark", colors[1]);
}
applyAccent();
window.addEventListener("focus", applyAccent);

try { setSystemLocale(await invoke("system_locale")); } catch {}
settings = await invoke("get_settings");
updatesEnabled = await invoke("updates_enabled");
snapshot = await invoke("get_usage");
trails = await invoke("get_sparklines").catch(() => ({})) ?? {};
await render();
await loadSubscriptions();
await loadHealth();
let page = "accounts";
try { page = localStorage.getItem("settings-page") || page; } catch {}
// Asked for a page while this window was being created (it is destroyed when closed).
page = (await invoke("take_settings_page").catch(() => null)) || page;
showPage(page);
if (page === "usage") loadUsage();
// Another computer's sign-ins are read from its roster, which can be slow: waited for briefly.
await Promise.race([firstPeerRuns, new Promise((done) => setTimeout(done, 4000))]);
delete document.documentElement.dataset.loading;
