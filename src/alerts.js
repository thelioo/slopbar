// Windows notifications for usage milestones, limits coming back, new usage-limit resets and
// automatic switches. Runs in the taskbar widget, which is always alive; what was already said is
// remembered across restarts so nothing is repeated, and the first look at an account only
// records where it stands.
import { t, resetsIn, windowLabel, resetText, groupResets } from "./i18n.js";

const { invoke } = window.__TAURI__.core;

const MILESTONES = [75, 90, 100];
const STORE = "slopbar-alerts";
const PROVIDER_NAME = { claude: "Claude", codex: "Codex" };
/** A reset time that moves by more than this means a new window, not a clock adjustment. */
const SAME_WINDOW_MS = 10 * 60 * 1000;

let state = { windows: {}, grants: {} };
try { state = { ...state, ...JSON.parse(localStorage.getItem(STORE) ?? "{}") }; } catch {}
const save = () => { try { localStorage.setItem(STORE, JSON.stringify(state)); } catch {} };

const milestone = (p) => MILESTONES.filter((m) => p >= m).pop() ?? 0;
const accountName = (a) => a.alias || a.email || t("unknownAccount");
const notify = (title, body) => invoke("notify", { title, body }).catch(() => {});

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

/** An automatic switch (see the balancer) is worth a notification; a manual one isn't. */
export function checkSwitch(event, prefs) {
  if (event.auto && prefs.switches) {
    const provider = PROVIDER_NAME[event.provider];
    notify(provider, t("switchedAuto", { provider, to: event.to ?? "" }));
  }
}
