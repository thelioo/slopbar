// An account in words, shared by the flyout and Settings so both say the same thing the same way:
// the plan's renewal, verdict and advice (from get_subscriptions), and the flyout's full tooltip,
// which carries everything the full card shows for an account in the one-line layout.
import { t, duration, shortDate, shortDay, dayTime, windowLabel, windowResetText, visibleResetWindows,
  placesText, errorText, creditsText, groupResets, resetText } from "./i18n.js";
import { sparklineText } from "./sparkline.js";

export const money = (v) => (v == null ? "" : `$${Number(v).toLocaleString(undefined, { maximumFractionDigits: 2 })}`);
export const pct = (v) => `${Math.round(v)}%`;
export const amount = (v) => Number(v).toLocaleString(undefined, { maximumFractionDigits: 2 });

export function renewalText(e) {
  if (e.ends_at) return t("subEnds", { d: shortDate(e.ends_at) });
  if (!e.renewal) return e.plan_kind === "personal" ? t("subRenewUnknown") : "";
  // An entered billing day is a calendar day; the others are moments.
  const when = e.renewal.source === "entered" ? shortDay(e.renewal.at) : shortDate(e.renewal.at);
  // Calendar days, as people count them: tomorrow is 1 day away whatever the hour.
  const midnight = (d) => { const x = new Date(d); x.setHours(0, 0, 0, 0); return x; };
  const days = Math.max(0, Math.round((midnight(e.renewal.at) - midnight(Date.now())) / 86400000));
  const source = t({ entered: "subSourceEntered", provider: "subSourceProvider", estimated: "subSourceEstimated" }[e.renewal.source]);
  return t(days === 0 ? "subRenewsToday" : days === 1 ? "subRenewsTomorrow" : "subRenews", { d: when, n: days, source });
}

export function adviceText(e) {
  const a = e.assessment;
  if (e.ends_at) return t("subAdviceEnding", { d: shortDate(e.ends_at) });
  if (e.cancel_after) return t("subLapsing", { d: shortDay(e.cancel_after) });
  if (e.plan_kind !== "personal") return "";
  if (a.verdict === "unused") return t(e.renewal ? "subAdviceUnused" : "subAdviceUnusedNoDate", { d: e.renewal ? (e.renewal.source === "entered" ? shortDay : shortDate)(e.renewal.at) : "" });
  if (a.verdict === "underused") return t(a.renew_wasteful ? "subAdviceUnderused" : "subAdviceUnderusedMaybe");
  if (a.verdict === "exhausted_weekly") return t("subAdviceExhausted");
  return "";
}

// Purchased Codex credits, never priced: OpenAI does not publish one credit price.
export function creditsLine(c) {
  if (!c) return "";
  if (c.unlimited) return t("subCreditsUnlimited");
  const parts = [];
  if (c.balance != null) parts.push(t("subCredits", { v: amount(c.balance) }));
  if (c.spent && c.since) parts.push(t("subCreditsSpent", { n: amount(c.spent), d: shortDate(c.since) }));
  return parts.join(" · ");
}

// Claude extra usage this month, when turned on: "Extra usage: $73.47 of $400 (18%)".
export function extraUsageLine(x) {
  if (!x || x.enabled !== true) return "";
  const used = x.used != null ? money(x.used) : "—";
  return x.monthly_limit != null
    ? t("subExtraUsage", { v: used, limit: money(x.monthly_limit), p: x.utilization != null ? pct(x.utilization) : "—" })
    : t("subExtraUsageNoLimit", { v: used });
}

const PROVIDER_NAME = { claude: "Claude", codex: "Codex" };
const ago = (iso, now) => duration(Math.max(1, Math.round((now - new Date(iso)) / 60000)));

/**
 * Everything known about an account, as groups of lines (empty groups and fields left out):
 * who it is, where it is in use, each usage window with its reset, the forecast, how current the
 * reading is and whether monitoring works, credits and resets held, and the plan's renewal,
 * cost and verdict. `sub` is the account's get_subscriptions entry, `trail` its sparkline
 * readings and `creditAdvice` the spend-or-switch line, when there are any.
 */
export function accountTipLines(a, { sub = null, trail = null, creditAdvice = "", now = Date.now() } = {}) {
  const provider = PROVIDER_NAME[a.provider] ?? a.provider;
  const groups = [];

  const name = a.alias || a.email || t("unknownAccount");
  const places = placesText(a);
  groups.push([
    a.alias && a.email ? `${name} (${a.email})` : name,
    [provider, a.plan, a.org].filter(Boolean).join(" · "),
    places.inUse ? (a.active ? `${t("inUse")}: ${places.inUse}` : places.inUse) : "",
    ...places.seen,
    a.used_outside ? t("usedOutside") : "",
    a.leased ? t("leased", { purpose: a.leased }) : "",
  ]);

  // Every window with its reset, model-specific ones included, worded as on the full card.
  const timed = new Set(visibleResetWindows(a.windows ?? []));
  const week = sub?.this_week;
  groups.push([
    ...(a.windows ?? []).map((w) => `${windowLabel(w)}: ${Math.round(w.used_percent)}% ${t("used")}`
      + (timed.has(w) ? ` · ${windowResetText(w, a.stale, a.fetched_at)}` : "")),
    a.runs_out_at && !a.stale ? t("runsOut", { at: dayTime(a.runs_out_at) }) : "",
    week?.pace_percent != null && !a.stale ? t("tipPace", { p: Math.round(week.pace_percent) }) : "",
    trail?.length ? sparklineText(trail) : "",
  ]);

  const problem = a.error || a.monitor_error;
  groups.push([
    a.fetched_at ? t("lastObserved", { t: ago(a.fetched_at, now) }) + (a.stale ? ` · ${t("lastReading")}` : "") : "",
    problem ? errorText(provider, problem) : "",
    t(a.monitor_connected ? "tipMonitorOn" : "tipMonitorOff"),
  ]);

  groups.push([
    a.credits ? creditsText(a.credits) : "",
    sub ? creditsLine(sub.credits) : "",
    ...groupResets(a.resets).map(resetText),
    creditAdvice,
    extraUsageLine(sub?.extra_usage ?? a.extra_usage),
  ]);

  if (sub) {
    const paid = sub.plan_kind === "personal";
    const cost = sub.monthly_cost != null ? t(sub.cost_entered ? "subCostEntered" : "subCostList", { v: money(sub.monthly_cost) }) : "";
    const renewal = paid || sub.ends_at ? renewalText(sub) : "";
    const verdict = sub.assessment?.verdict;
    groups.push([
      [renewal, cost].filter(Boolean).join(" · "),
      verdict && verdict !== "insufficient_data" ? t("tipVerdict", { v: t(`subVerdict_${verdict}`) }) : "",
      sub.idle_now ? t("subIdleNow") : "",
      sub.delinquent ? t("subDelinquent") : "",
      adviceText(sub),
    ]);
  }
  return groups.map((g) => g.filter(Boolean)).filter((g) => g.length);
}

/** The tooltip itself: one line each, a blank line between groups. */
export const accountTip = (a, extra) => accountTipLines(a, extra).map((g) => g.join("\n")).join("\n\n");
