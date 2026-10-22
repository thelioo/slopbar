// Windows notifications for usage milestones, limits coming back, new usage-limit resets, automatic
// switches, and use none of the user's computers accounts for. Runs in the taskbar widget, which is always alive; what was already said is
// remembered across restarts so nothing is repeated, and the first look at an account only
// records where it stands.
import { t, resetsIn, windowLabel, resetText, groupResets, duration, dayTime, shortDate, LOCALE } from "./i18n.js";

const { invoke } = window.__TAURI__.core;

const MILESTONES = [75, 90, 100];
const STORE = "slopbar-alerts";
const PROVIDER_NAME = { claude: "Claude", codex: "Codex" };
/** A reset time that moves by more than this means a new window, not a clock adjustment. */
const SAME_WINDOW_MS = 10 * 60 * 1000;
const UNEXPECTED_FRESH_MS = 60 * 60 * 1000;

let state = { windows: {}, grants: {}, unexpected: {} };
try { state = { ...state, ...JSON.parse(localStorage.getItem(STORE) ?? "{}") }; } catch {}
const save = () => { try { localStorage.setItem(STORE, JSON.stringify(state)); } catch {} };

const milestone = (p) => MILESTONES.filter((m) => p >= m).pop() ?? 0;
const accountName = (a) => a.alias || a.email || t("unknownAccount");
const notify = (title, body, page = null) => invoke("notify", { title, body, page }).catch(() => {});

function newWindow(before, w) {
  if (!before.resets_at) return false;
  if (new Date(before.resets_at) <= Date.now()) return true;
  return !w.resets_at || Math.abs(new Date(w.resets_at) - new Date(before.resets_at)) > SAME_WINDOW_MS;
}

/** Compares a fresh snapshot with what was seen before and notifies what changed. */
export function checkUsage(snap, prefs) {
  for (const a of snap.accounts) {
    if (a.stale || a.error) continue;
    const provider = PROVIDER_NAME[a.provider];
    for (const w of a.windows) {
      const key = `${a.provider}:${a.id}:${w.kind}:${w.window_seconds ?? ""}`;
      const before = state.windows[key];
      const now = milestone(w.used_percent);
      state.windows[key] = { resets_at: w.resets_at, milestone: before && !newWindow(before, w) ? Math.max(before.milestone, now) : now };
      if (!before) continue;
      const details = [accountName(a), windowLabel(w), w.resets_at ? resetsIn(w.resets_at) : ""].filter(Boolean).join(" · ");
      if (newWindow(before, w)) {
        if (prefs.resets && before.milestone >= 90 && now < before.milestone) {
          notify(t("nBackTitle", { provider }), t("nBackBody", { window: windowLabel(w), account: accountName(a) }));
        }
      } else if (prefs.usage && now > before.milestone) {
        const usable = a.resets?.some((r) => r.usable && r.count > 0);
        if (now >= 100) notify(t("nLimitTitle", { provider }), [details, usable ? t("nLimitHasReset") : ""].filter(Boolean).join("\n"));
        else notify(t("nUsageTitle", { provider, p: now }), details);
      }
    }
    const key = `${a.provider}:${a.id}`;
    // The app decides when unexpected use is worth a notification (once per six hours); a new
    // alert_at is that decision. One from long ago (SlopBar was closed) is not news any more.
    const alertAt = a.unexpected?.alert_at;
    if (alertAt && state.unexpected?.[key] !== alertAt) {
      state.unexpected = { ...state.unexpected, [key]: alertAt };
      if (prefs.unexpected_use && Date.now() - new Date(alertAt) < UNEXPECTED_FRESH_MS) {
        invoke("notify_unexpected", {
          provider: a.provider, title: t("nUnexpectedTitle", { provider: PROVIDER_NAME[a.provider] }),
          body: t("nUnexpectedBody", { account: accountName(a) }), review: t("nUnexpectedReview"), security: t("nUnexpectedSecurity"),
        }).catch(() => {});
      }
    }
    const total = (a.resets ?? []).reduce((n, r) => n + r.count, 0);
    const before = state.grants[key];
    state.grants[key] = total;
    if (prefs.resets && before !== undefined && total > before) {
      const grant = groupResets(a.resets)[0];
      notify(t("nGrantTitle", { provider }), `${accountName(a)} · ${resetText(grant)}`);
    }
  }
  save();
}

const until = (iso) => duration(Math.max(1, Math.round((new Date(iso) - Date.now()) / 60000)));
const money = (v) => `$${Number(v).toLocaleString(undefined, { maximumFractionDigits: 2 })}`;

/**
 * Plan renewals, the account in use running out of its week, and Codex Pro 200 accounts whose use
 * would not fit the plan's smaller allowance after its cut. The app decides what is due and
 * remembers what was shown (it honours the notification settings); this only words it. Clicking
 * one opens Settings > Subscriptions.
 */
export async function checkPlans() {
  let due;
  try { due = await invoke("take_plan_alerts"); } catch { return; }
  for (const a of due ?? []) {
    const provider = PROVIDER_NAME[a.provider];
    if (a.kind === "runs_out") {
      notify(t("nRunsOutTitle", { provider, at: dayTime(a.at) }), t("nRunsOutBody", { account: accountName(a), t: until(a.resets_at) }), "subscriptions");
      continue;
    }
    if (a.kind === "extra_usage") {
      notify(t("nExtraTitle", { provider, p: Math.round(a.percent) }),
        t("nExtraBody", { account: accountName(a), v: money(a.used ?? 0), limit: a.limit != null ? money(a.limit) : "—" }), "subscriptions");
      continue;
    }
    if (a.kind === "pro_allowance_cut") {
      // The last day of the old allowance, at local noon so no time zone moves it.
      notify(t("nProCutTitle", { d: shortDate(`${a.at.slice(0, 10)}T12:00:00`) }), t("nProCutBody", { accounts: (a.accounts ?? []).join(", ") }), "subscriptions");
      continue;
    }
    const body = [accountName(a), a.kind === "renewal_wasteful" && a.verdict ? t(`subVerdict_${a.verdict}`) : "",
      a.monthly_cost != null ? t("perMonth", { v: money(a.monthly_cost) }) : ""].filter(Boolean).join(" · ");
    notify(t("nRenewTitle", { provider, t: until(a.at) }), a.kind === "renewal_wasteful" ? `${body}\n${t("nRenewWasteHint")}` : body, "subscriptions");
  }
}

// The weekday of a renewal: an entered date is a calendar day (midnight UTC), the rest are moments.
const renewDay = (r) => new Date(r.at).toLocaleDateString(LOCALE, { weekday: "short", ...(r.source === "entered" ? { timeZone: "UTC" } : {}) });

/**
 * The Monday digest: spend, possible savings, renewals this week, accounts that ran out and how
 * many plans the usage needs. The app decides when it is due (Monday mornings, once a week, when
 * turned on); this only words it. Clicking it opens Settings > Subscriptions.
 */
export async function checkDigest() {
  let d;
  try { d = await invoke("take_digest"); } catch { return; }
  if (!d) return;
  const spend = [t("nDigestSpend", { v: money(d.monthly_total) })];
  if (d.unpriced) spend.push(t("nDigestUnpriced", { n: d.unpriced }));
  if (d.possible_savings > 0) spend.push(t("nDigestSave", { v: money(d.possible_savings) }));
  const lines = [spend.join(" · ")];
  if (d.renewing.length) lines.push(t("nDigestRenewing", { list: d.renewing.map((r) => `${r.name} (${renewDay(r)})`).join(", ") }));
  if (d.ran_out.length) lines.push(t("nDigestRanOut", { list: d.ran_out.join(", ") }));
  for (const p of d.pools) {
    lines.push(t("nDigestPool", { provider: PROVIDER_NAME[p.provider], n: p.plans_needed, unit: p.unit, held: Math.round(p.plans_held * 10) / 10 }));
  }
  notify(t("nDigestTitle"), lines.join("
"), "subscriptions");
}

/** An automatic switch (see the balancer) is worth a notification; a manual one isn't. */
export function checkSwitch(event, prefs) {
  if (event.auto && prefs.switches) {
    const provider = PROVIDER_NAME[event.provider];
    notify(provider, t("switchedAuto", { provider, to: event.to ?? "" }));
  }
}
