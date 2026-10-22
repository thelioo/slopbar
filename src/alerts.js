// Words the spend-or-switch advice the flyout shows for a Codex account at its limit with a
// reset credit. Notifications (usage milestones, limits back, new resets, switches, plans, the
// digest, runaway agents, reset credits) are sent by the app itself: src-tauri/src/alerts.rs,
// which keeps the wording that used to live here.
import { t, duration, dayTime } from "./i18n.js";

const accountName = (a) => a.alias || a.email || t("unknownAccount");
const until = (iso) => duration(Math.max(1, Math.round((new Date(iso) - Date.now()) / 60000)));

/** What to do about a Codex account at its limit with a reset credit (see reset_credits.rs). */
export function creditAdviceText(a) {
  if (a.choice === "switch") return t("nCreditSwitch", { to: a.switch_to ? accountName(a.switch_to) : "", p: `${Math.round(a.switch_to?.left_percent ?? 0)}%` });
  if (a.choice === "wait") return t("nCreditWait", { t: a.resets_at ? until(a.resets_at) : "" });
  return a.reason === "expires_soon" && a.expires_at ? t("nCreditSpendExpires", { d: dayTime(a.expires_at) }) : t("nCreditSpendNoOther");
}
