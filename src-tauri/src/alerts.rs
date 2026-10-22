//! Windows notifications for usage milestones, limits coming back, new usage-limit resets,
//! automatic switches, use none of the user's computers accounts for, plan renewals, the Monday
//! digest, runaway agents and reset credits. Sent by the app itself after every refresh, so they
//! arrive whatever widget is showing and with no page open (they used to come from the taskbar
//! widget's page, src/alerts.js, whose wording this keeps word for word).
//!
//! What was already said is remembered across restarts (alerts.json in the app's folder) so
//! nothing is repeated, and the first look at an account only records where it stands. Plans,
//! the digest and capacity alerts are decided elsewhere (`plan_alerts`, `digest`, `burn`,
//! `reset_credits`), which remember what they returned; this only words them.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::Provider;
use crate::i18n::t;
use crate::i18n_format as fmt;
use crate::model::{AccountUsage, Snapshot, UsageWindow};
use crate::phone::Kind as PhoneKind;
use crate::settings::Notifications;

const MILESTONES: [u32; 3] = [75, 90, 100];
/// A reset time that moves by more than this means a new window, not a clock adjustment.
const SAME_WINDOW_MS: i64 = 10 * 60 * 1000;
const UNEXPECTED_FRESH_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// What a usage window stood at when last seen.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WindowSeen {
    pub resets_at: Option<DateTime<Utc>>,
    pub milestone: u32,
}

/// What was already seen and said, keyed `provider:id[:kind:window_seconds]`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub windows: HashMap<String, WindowSeen>,
    /// Usage-limit resets each account held.
    pub grants: HashMap<String, u32>,
    /// The unexpected-use decision (`alert_at`) last acted on.
    pub unexpected: HashMap<String, DateTime<Utc>>,
}

pub fn load_from(path: &Path) -> State {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_to(path: &Path, state: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    crate::private_store::write(path, &bytes).map_err(|e| e.to_string())
}

/// The same news for the phone, naming only the provider (see `phone`).
#[derive(Debug, Clone, PartialEq)]
pub struct Phone {
    pub kind: PhoneKind,
    pub title: String,
    pub body: String,
}

/// A notification to show.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// With `page` (a Settings page), clicking it opens Settings there.
    Plain { title: String, body: String, page: Option<&'static str>, phone: Option<Phone> },
    /// Unexpected use, with buttons to review the account or open the provider's security page.
    Unexpected { provider: Provider, title: String, body: String, review: String, security: String, phone: Option<Phone> },
}

fn plain(title: String, body: String) -> Notice {
    Notice::Plain { title, body, page: None, phone: None }
}

fn provider_name(p: &str) -> &str {
    match p { "claude" => "Claude", "codex" => "Codex", other => other }
}

/// The phone's wording for a kind of news (alerts.js `phoneForward`).
pub fn phone_forward(lang: &str, kind: PhoneKind, provider: &str) -> Phone {
    let name = provider_name(provider);
    let key = match kind {
        PhoneKind::Renewal => "Renew",
        PhoneKind::UnexpectedUse => "Unexpected",
        PhoneKind::Runaway => "Runaway",
        PhoneKind::LimitReached | PhoneKind::Test => "Limit",
    };
    Phone { kind, title: t(lang, &format!("p{key}Title"), &[("provider", name)]), body: t(lang, &format!("p{key}Body"), &[("provider", name)]) }
}

fn provider_str(p: Provider) -> &'static str {
    match p { Provider::Claude => "claude", Provider::Codex => "codex" }
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

fn account_name(lang: &str, a: &AccountUsage) -> String {
    nonempty(a.alias.as_deref()).or(nonempty(a.email.as_deref())).map(str::to_string).unwrap_or_else(|| t(lang, "unknownAccount", &[]))
}

/// alias || email || "Unknown account", for the decided alerts (JSON as the frontend got it).
fn value_name(lang: &str, v: &Value) -> String {
    let s = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    s("alias").or(s("email")).map(str::to_string).unwrap_or_else(|| t(lang, "unknownAccount", &[]))
}

fn milestone(p: f64) -> u32 {
    MILESTONES.iter().copied().filter(|m| p >= *m as f64).last().unwrap_or(0)
}

fn new_window(before: &WindowSeen, w: &UsageWindow, now: DateTime<Utc>) -> bool {
    let Some(was) = before.resets_at else { return false };
    if was <= now {
        return true;
    }
    w.resets_at.is_none_or(|at| (at - was).num_milliseconds().abs() > SAME_WINDOW_MS)
}

/// Compares a fresh snapshot with what was seen before and returns what changed worth saying
/// (alerts.js `checkUsage`). `state` is updated whatever the settings say.
pub fn check_usage(state: &mut State, snap: &Snapshot, prefs: &Notifications, lang: &str, now: DateTime<Utc>) -> Vec<Notice> {
    let mut out = Vec::new();
    for a in &snap.accounts {
        if a.stale || a.error.is_some() {
            continue;
        }
        let pstr = provider_str(a.provider);
        let provider = provider_name(pstr);
        let name = account_name(lang, a);
        for w in &a.windows {
            let key = format!("{pstr}:{}:{}:{}", a.id, w.kind, w.window_seconds.map(|s| s.to_string()).unwrap_or_default());
            let before = state.windows.get(&key).cloned();
            let now_m = milestone(w.used_percent);
            let renewed = before.as_ref().is_some_and(|b| new_window(b, w, now));
            let kept = match &before { Some(b) if !renewed => b.milestone.max(now_m), _ => now_m };
            state.windows.insert(key, WindowSeen { resets_at: w.resets_at, milestone: kept });
            let Some(before) = before else { continue };
            let label = fmt::window_label(lang, &w.kind, w.window_seconds);
            if renewed {
                if prefs.resets && before.milestone >= 90 && now_m < before.milestone {
                    out.push(plain(t(lang, "nBackTitle", &[("provider", provider)]), t(lang, "nBackBody", &[("window", &label), ("account", &name)])));
                }
            } else if prefs.usage && now_m > before.milestone {
                let resets = w.resets_at.map(|r| fmt::resets_in(lang, Some(r), now)).unwrap_or_default();
                let details = [name.as_str(), label.as_str(), resets.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                if now_m >= 100 {
                    let usable = a.resets.iter().any(|r| r.usable && r.count > 0);
                    let hint = if usable { t(lang, "nLimitHasReset", &[]) } else { String::new() };
                    let body = [details, hint].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
                    out.push(Notice::Plain {
                        title: t(lang, "nLimitTitle", &[("provider", provider)]), body, page: None,
                        phone: Some(phone_forward(lang, PhoneKind::LimitReached, pstr)),
                    });
                } else {
                    out.push(plain(t(lang, "nUsageTitle", &[("provider", provider), ("p", &now_m.to_string())]), details));
                }
            }
        }
        let key = format!("{pstr}:{}", a.id);
        // The app decides when unexpected use is worth a notification (once per six hours); a new
        // alert_at is that decision. One from long ago (SlopBar was closed) is not news any more.
        if let Some(alert_at) = a.unexpected.as_ref().and_then(|u| u.alert_at) {
            if state.unexpected.get(&key) != Some(&alert_at) {
                state.unexpected.insert(key.clone(), alert_at);
                if prefs.unexpected_use && (now - alert_at).num_milliseconds() < UNEXPECTED_FRESH_MS {
                    out.push(Notice::Unexpected {
                        provider: a.provider,
                        title: t(lang, "nUnexpectedTitle", &[("provider", provider)]),
                        body: t(lang, "nUnexpectedBody", &[("account", &name)]),
                        review: t(lang, "nUnexpectedReview", &[]),
                        security: t(lang, "nUnexpectedSecurity", &[]),
                        phone: Some(phone_forward(lang, PhoneKind::UnexpectedUse, pstr)),
                    });
                }
            }
        }
        let total: u32 = a.resets.iter().map(|r| r.count).sum();
        let before = state.grants.insert(key, total);
        if prefs.resets && before.is_some_and(|b| total > b) {
            if let Some(grant) = fmt::group_resets(&a.resets).first() {
                out.push(plain(t(lang, "nGrantTitle", &[("provider", provider)]), format!("{name} · {}", fmt::reset_text(lang, grant, now))));
            }
        }
    }
    out
}

fn time(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    v.get(key).and_then(Value::as_str).and_then(|s| s.parse().ok())
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn subscriptions(title: String, body: String, phone: Option<Phone>) -> Notice {
    Notice::Plain { title, body, page: Some("subscriptions"), phone }
}

/// Plan renewals, the account in use running out of its week, extra usage, and Codex Pro 200
/// accounts whose use would not fit after the allowance cut (alerts.js `checkPlans`). `due` is
/// what `plan_alerts::due` returned, as JSON.
pub fn plan_notices(lang: &str, due: &[Value], now: DateTime<Utc>) -> Vec<Notice> {
    let mut out = Vec::new();
    for a in due {
        let pstr = text(a, "provider");
        let provider = provider_name(pstr);
        let name = value_name(lang, a);
        let until = |key: &str| time(a, key).map(|at| fmt::until(lang, at, now)).unwrap_or_default();
        match text(a, "kind") {
            "runs_out" => {
                let at = time(a, "at").map(|at| fmt::day_time(lang, at)).unwrap_or_default();
                out.push(subscriptions(t(lang, "nRunsOutTitle", &[("provider", provider), ("at", &at)]),
                    t(lang, "nRunsOutBody", &[("account", &name), ("t", &until("resets_at"))]), None));
            }
            "extra_usage" => {
                let p = fmt::js_number(fmt::js_round(num(a, "percent").unwrap_or(0.0)));
                let limit = num(a, "limit").map(|l| fmt::money(lang, l)).unwrap_or_else(|| "—".into());
                out.push(subscriptions(t(lang, "nExtraTitle", &[("provider", provider), ("p", &p)]),
                    t(lang, "nExtraBody", &[("account", &name), ("v", &fmt::money(lang, num(a, "used").unwrap_or(0.0))), ("limit", &limit)]), None));
            }
            "pro_allowance_cut" => {
                // The last day of the old allowance, as a calendar day so no time zone moves it.
                let d = text(a, "at").get(..10).and_then(|s| s.parse().ok()).map(|d| fmt::short_date_ymd(lang, d)).unwrap_or_default();
                let accounts: Vec<&str> = a.get("accounts").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                out.push(subscriptions(t(lang, "nProCutTitle", &[("d", &d)]), t(lang, "nProCutBody", &[("accounts", &accounts.join(", "))]), None));
            }
            kind => {
                let wasteful = kind == "renewal_wasteful";
                let verdict = match a.get("verdict").and_then(Value::as_str) {
                    Some(v) if wasteful && !v.is_empty() => t(lang, &format!("subVerdict_{v}"), &[]),
                    _ => String::new(),
                };
                let cost = num(a, "monthly_cost").map(|c| t(lang, "perMonth", &[("v", &fmt::money(lang, c))])).unwrap_or_default();
                let body = [name, verdict, cost].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                let body = if wasteful { format!("{body}\n{}", t(lang, "nRenewWasteHint", &[])) } else { body };
                // Only a renewal within a day goes to the phone.
                let tomorrow = time(a, "at").is_some_and(|at| (at - now).num_milliseconds() <= DAY_MS + 60 * 60 * 1000);
                out.push(subscriptions(t(lang, "nRenewTitle", &[("provider", provider), ("t", &until("at"))]), body,
                    tomorrow.then(|| phone_forward(lang, PhoneKind::Renewal, pstr))));
            }
        }
    }
    out
}

/// The Monday digest (alerts.js `checkDigest`); `d` is `digest::Digest` as JSON.
pub fn digest_notice(lang: &str, d: &Value) -> Notice {
    let mut spend = vec![t(lang, "nDigestSpend", &[("v", &fmt::money(lang, num(d, "monthly_total").unwrap_or(0.0)))])];
    let unpriced = d.get("unpriced").and_then(Value::as_u64).unwrap_or(0);
    if unpriced > 0 {
        spend.push(t(lang, "nDigestUnpriced", &[("n", &unpriced.to_string())]));
    }
    let savings = num(d, "possible_savings").unwrap_or(0.0);
    if savings > 0.0 {
        spend.push(t(lang, "nDigestSave", &[("v", &fmt::money(lang, savings))]));
    }
    let mut lines = vec![spend.join(" · ")];
    let list = |k: &str| d.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let renewing = list("renewing");
    if !renewing.is_empty() {
        // The weekday of a renewal: an entered date is a calendar day (midnight UTC), the rest are moments.
        let items: Vec<String> = renewing.iter().map(|r| {
            let day = time(r, "at").map(|at| fmt::weekday(lang, at, text(r, "source") == "entered")).unwrap_or_default();
            format!("{} ({day})", text(r, "name"))
        }).collect();
        lines.push(t(lang, "nDigestRenewing", &[("list", &items.join(", "))]));
    }
    let ran_out: Vec<String> = list("ran_out").iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    if !ran_out.is_empty() {
        lines.push(t(lang, "nDigestRanOut", &[("list", &ran_out.join(", "))]));
    }
    for p in list("pools") {
        let held = fmt::js_number(fmt::js_round(num(&p, "plans_held").unwrap_or(0.0) * 10.0) / 10.0);
        let needed = p.get("plans_needed").map(|n| n.to_string()).unwrap_or_default();
        lines.push(t(lang, "nDigestPool", &[("provider", provider_name(text(&p, "provider"))), ("n", &needed), ("unit", text(&p, "unit")), ("held", &held)]));
    }
    subscriptions(t(lang, "nDigestTitle", &[]), lines.join("\n"), None)
}

/// What to do about a Codex account at its limit with a reset credit (alerts.js
/// `creditAdviceText`); `a` is a `reset_credits::CreditAdvice` as JSON.
pub fn credit_advice_text(lang: &str, a: &Value, now: DateTime<Utc>) -> String {
    match text(a, "choice") {
        "switch" => {
            let to = a.get("switch_to").filter(|v| !v.is_null());
            let name = to.map(|v| value_name(lang, v)).unwrap_or_default();
            let left = to.and_then(|v| num(v, "left_percent")).unwrap_or(0.0);
            t(lang, "nCreditSwitch", &[("to", &name), ("p", &format!("{}%", fmt::js_number(fmt::js_round(left))))])
        }
        "wait" => t(lang, "nCreditWait", &[("t", &time(a, "resets_at").map(|at| fmt::until(lang, at, now)).unwrap_or_default())]),
        _ => match time(a, "expires_at") {
            Some(e) if text(a, "reason") == "expires_soon" => t(lang, "nCreditSpendExpires", &[("d", &fmt::day_time(lang, e))]),
            _ => t(lang, "nCreditSpendNoOther", &[]),
        },
    }
}

/// Runaway agents and Codex reset credits (alerts.js `checkCapacity`); `due` is
/// `{ burn: [BurnAlert], credits: [CreditAlert] }`.
pub fn capacity_notices(lang: &str, due: &Value, now: DateTime<Utc>) -> Vec<Notice> {
    let mut out = Vec::new();
    let list = |k: &str| due.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    for b in list("burn") {
        let this_pc = b.get("this_pc").filter(|v| !v.is_null());
        let first = if text(&b, "trigger") == "weekly_rise" {
            let p = fmt::js_number(fmt::js_round(num(&b, "rise_points").unwrap_or(0.0)));
            let m = b.get("rise_minutes").and_then(Value::as_i64).map(|m| m.to_string()).unwrap_or_default();
            t(lang, "nBurnRise", &[("p", &p), ("m", &m)])
        } else {
            let x = this_pc.and_then(|pc| {
                let base = num(pc, "baseline").filter(|b| *b != 0.0)?;
                Some(fmt::js_number(fmt::js_round(num(pc, "tokens").unwrap_or(0.0) / base)))
            }).unwrap_or_default();
            t(lang, "nBurnLocal", &[("x", &x)])
        };
        let second = match time(&b, "limit_at") {
            Some(at) => t(lang, "nBurnLimit", &[("at", &fmt::day_time(lang, at))]),
            None => t(lang, "nBurnLasts", &[]),
        };
        let mut body = vec![format!("{first} · {second}")];
        if let Some(pc) = this_pc {
            body.push(t(lang, "nBurnThisPc", &[("repo", text(pc, "repo")), ("model", text(pc, "model"))]));
        }
        out.push(Notice::Plain {
            title: t(lang, "nBurnTitle", &[("provider", provider_name(text(&b, "provider"))), ("account", &value_name(lang, &b))]),
            body: body.join("\n"), page: Some("usage"), phone: None,
        });
    }
    for c in list("credits") {
        let provider = provider_name(text(&c, "provider"));
        let name = value_name(lang, &c);
        if text(&c, "kind") == "expiring" {
            let d = time(&c, "expires_at").map(|e| fmt::day_time(lang, e)).unwrap_or_default();
            let n = c.get("count").map(|n| n.to_string()).unwrap_or_default();
            out.push(plain(t(lang, "nCreditExpiringTitle", &[("provider", provider), ("d", &d)]), t(lang, "nCreditExpiringBody", &[("account", &name), ("n", &n)])));
        } else {
            out.push(plain(t(lang, "nCreditTitle", &[("provider", provider), ("account", &name)]), credit_advice_text(lang, &c, now)));
        }
    }
    out
}

/// An automatic switch (see the balancer) is worth a notification; a manual one isn't.
pub fn switch_notice(lang: &str, provider: Provider, to: Option<&str>, auto: bool, prefs: &Notifications) -> Option<Notice> {
    (auto && prefs.switches).then(|| {
        let name = provider_name(provider_str(provider));
        plain(name.to_string(), t(lang, "switchedAuto", &[("provider", name), ("to", to.unwrap_or(""))]))
    })
}

/// An automatic monitoring sign-in started (see `auto_monitor`): the user finishes it in the browser.
pub fn monitor_auto_notice(lang: &str, name: Option<&str>) -> Notice {
    let unknown = t(lang, "unknownAccount", &[]);
    plain(t(lang, "monitorAutoTitle", &[("name", name.unwrap_or(&unknown))]), t(lang, "monitorAutoBody", &[]))
}

#[cfg(feature = "desktop")]
pub use glue::{on_monitor_auto_started, on_snapshot, on_switch};

#[cfg(feature = "desktop")]
mod glue {
    use std::sync::{Mutex, OnceLock};

    use tauri::{AppHandle, Manager};

    use super::*;
    use crate::AppState;

    fn path() -> Option<std::path::PathBuf> {
        crate::settings::app_dir().map(|d| d.join("alerts.json"))
    }

    fn store() -> &'static Mutex<State> {
        static STORE: OnceLock<Mutex<State>> = OnceLock::new();
        STORE.get_or_init(|| Mutex::new(path().map(|p| load_from(&p)).unwrap_or_default()))
    }

    /// The notification settings and the language they are worded in.
    fn context(app: &AppHandle) -> (Notifications, String) {
        let settings = app.state::<AppState>().settings.lock().unwrap().clone();
        (settings.notifications, crate::i18n::resolve(&settings.language))
    }

    fn send(app: &AppHandle, notices: Vec<Notice>) {
        let forward = |p: Option<Phone>| p.map(|p| crate::notify::PhoneForward { kind: p.kind, title: p.title, body: p.body });
        for n in notices {
            let _ = match n {
                Notice::Plain { title, body, page, phone } => crate::notify::notify(app.clone(), title, body, page.map(str::to_string), forward(phone)),
                Notice::Unexpected { provider, title, body, review, security, phone } =>
                    crate::notify::notify_unexpected(app.clone(), provider, title, body, review, security, forward(phone)),
            };
        }
    }

    /// After a snapshot is published: usage milestones, plans, the digest and capacity alerts, off
    /// the caller's thread and one run at a time.
    pub fn on_snapshot(app: &AppHandle) {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut state = store().lock().unwrap_or_else(|e| e.into_inner());
            let shared = app.state::<AppState>();
            let snapshot = shared.snapshot.lock().unwrap().clone();
            if snapshot.updated_at.is_none() {
                return;
            }
            let (prefs, lang) = context(&app);
            let now = Utc::now();
            let before = state.clone();
            let mut notices = check_usage(&mut state, &snapshot, &prefs, &lang, now);
            if *state != before {
                if let Some(p) = path() { let _ = save_to(&p, &state); }
            }
            let plans = serde_json::to_value(crate::app::plan_alerts_due(&shared)).unwrap_or_default();
            notices.extend(plan_notices(&lang, plans.as_array().map(Vec::as_slice).unwrap_or(&[]), now));
            if let Some(d) = crate::app::digest_due(&shared) {
                notices.push(digest_notice(&lang, &serde_json::to_value(d).unwrap_or_default()));
            }
            notices.extend(capacity_notices(&lang, &crate::app::capacity_alerts_due(&shared), now));
            drop(state);
            send(&app, notices);
        });
    }

    /// Next to the "account-switched" event.
    pub fn on_switch(app: &AppHandle, provider: Provider, to: Option<&str>, auto: bool) {
        let (prefs, lang) = context(app);
        if let Some(n) = switch_notice(&lang, provider, to, auto, &prefs) {
            send(app, vec![n]);
        }
    }

    /// Next to the "monitoring-auto-started" event.
    pub fn on_monitor_auto_started(app: &AppHandle, name: Option<&str>) {
        let (_, lang) = context(app);
        send(app, vec![monitor_auto_notice(&lang, name)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ResetGrant, UnexpectedUse};
    use serde_json::json;

    fn at(s: &str) -> DateTime<Utc> { s.parse().unwrap() }
    const NOW: &str = "2026-10-22T10:00:00Z";

    fn account(used: f64, resets_at: &str) -> AccountUsage {
        serde_json::from_value(json!({
            "id": "a1", "provider": "claude", "email": "me@example.invalid", "alias": null, "plan": null, "org": null,
            "active": true, "sources": [], "credits": null, "resets": [], "error": null, "stale": false, "fetched_at": null,
            "monitor_connected": true,
            "windows": [{ "kind": "weekly", "window_seconds": null, "used_percent": used, "resets_at": resets_at }],
        })).unwrap()
    }

    fn snap(accounts: Vec<AccountUsage>) -> Snapshot {
        Snapshot { accounts, updated_at: Some(at(NOW)) }
    }

    fn titles(n: &[Notice]) -> Vec<String> {
        n.iter().map(|n| match n { Notice::Plain { title, .. } | Notice::Unexpected { title, .. } => title.clone() }).collect()
    }

    const RESET: &str = "2026-10-25T10:00:00Z";

    #[test]
    fn first_look_only_records() {
        let mut s = State::default();
        let out = check_usage(&mut s, &snap(vec![account(95.0, RESET)]), &Notifications::default(), "en", at(NOW));
        assert!(out.is_empty());
        assert_eq!(s.windows["claude:a1:weekly:"], WindowSeen { resets_at: Some(at(RESET)), milestone: 90 });
        assert_eq!(s.grants["claude:a1"], 0);
    }

    #[test]
    fn milestones_are_said_once_when_crossed() {
        let prefs = Notifications::default();
        let mut s = State::default();
        check_usage(&mut s, &snap(vec![account(50.0, RESET)]), &prefs, "en", at(NOW));
        let out = check_usage(&mut s, &snap(vec![account(76.0, RESET)]), &prefs, "en", at(NOW));
        assert_eq!(titles(&out), ["Claude at 75%"]);
        let Notice::Plain { body, phone, .. } = &out[0] else { panic!() };
        assert_eq!(body, "me@example.invalid · Weekly · Resets in 3d");
        assert!(phone.is_none());
        assert!(check_usage(&mut s, &snap(vec![account(80.0, RESET)]), &prefs, "en", at(NOW)).is_empty());
        let out = check_usage(&mut s, &snap(vec![account(100.0, RESET)]), &prefs, "en", at(NOW));
        assert_eq!(titles(&out), ["Claude limit reached"]);
        let Notice::Plain { phone, .. } = &out[0] else { panic!() };
        assert_eq!(phone.as_ref().unwrap().kind, PhoneKind::LimitReached);
        // Off in settings: nothing said, but still recorded.
        let mut quiet = State::default();
        let off = Notifications { usage: false, ..Notifications::default() };
        check_usage(&mut quiet, &snap(vec![account(10.0, RESET)]), &off, "en", at(NOW));
        assert!(check_usage(&mut quiet, &snap(vec![account(92.0, RESET)]), &off, "en", at(NOW)).is_empty());
        assert_eq!(quiet.windows["claude:a1:weekly:"].milestone, 90);
    }

    #[test]
    fn a_new_window_says_the_limit_is_back() {
        let prefs = Notifications::default();
        let mut s = State::default();
        check_usage(&mut s, &snap(vec![account(100.0, RESET)]), &prefs, "en", at(NOW));
        // A small clock adjustment is the same window.
        let same = "2026-10-25T10:05:00Z";
        assert!(check_usage(&mut s, &snap(vec![account(100.0, same)]), &prefs, "en", at(NOW)).is_empty());
        assert_eq!(s.windows["claude:a1:weekly:"].milestone, 100);
        let out = check_usage(&mut s, &snap(vec![account(3.0, "2026-11-01T10:00:00Z")]), &prefs, "en", at(NOW));
        assert_eq!(titles(&out), ["Claude is available again"]);
        let Notice::Plain { body, .. } = &out[0] else { panic!() };
        assert_eq!(body, "Weekly limit reset · me@example.invalid");
        assert_eq!(s.windows["claude:a1:weekly:"].milestone, 0);
    }

    #[test]
    fn a_passed_reset_is_a_new_window() {
        let before = WindowSeen { resets_at: Some(at("2026-10-22T09:00:00Z")), milestone: 90 };
        let w = UsageWindow { kind: "weekly".into(), window_seconds: None, used_percent: 90.0, resets_at: Some(at("2026-10-22T09:00:00Z")) };
        assert!(new_window(&before, &w, at(NOW)));
        assert!(!new_window(&WindowSeen::default(), &w, at(NOW)));
    }

    #[test]
    fn more_resets_are_announced() {
        let prefs = Notifications::default();
        let mut s = State::default();
        let mut a = account(10.0, RESET);
        check_usage(&mut s, &snap(vec![a.clone()]), &prefs, "en", at(NOW));
        a.resets = vec![ResetGrant { kind: "full".into(), title: None, count: 1, usable: true, expires_at: None, next_available_at: None }];
        let out = check_usage(&mut s, &snap(vec![a.clone()]), &prefs, "en", at(NOW));
        assert_eq!(titles(&out), ["New Claude limit reset"]);
        let Notice::Plain { body, .. } = &out[0] else { panic!() };
        assert_eq!(body, "me@example.invalid · Full reset · usable now");
        assert!(check_usage(&mut s, &snap(vec![a]), &prefs, "en", at(NOW)).is_empty());
    }

    #[test]
    fn unexpected_use_only_when_fresh_and_once() {
        let prefs = Notifications::default();
        let mut s = State::default();
        let mut a = account(10.0, RESET);
        a.unexpected = Some(UnexpectedUse { seen_week: 1, last_seen: at(NOW), alert_at: Some(at("2026-10-22T09:30:00Z")) });
        let out = check_usage(&mut s, &snap(vec![a.clone()]), &prefs, "en", at(NOW));
        assert!(matches!(&out[..], [Notice::Unexpected { provider: Provider::Claude, .. }]));
        assert!(check_usage(&mut s, &snap(vec![a.clone()]), &prefs, "en", at(NOW)).is_empty());
        // A decision from long ago (the app was closed) is recorded but not news.
        a.unexpected = Some(UnexpectedUse { seen_week: 1, last_seen: at(NOW), alert_at: Some(at("2026-10-22T08:00:00Z")) });
        assert!(check_usage(&mut s, &snap(vec![a]), &prefs, "en", at(NOW)).is_empty());
        assert_eq!(s.unexpected["claude:a1"], at("2026-10-22T08:00:00Z"));
    }

    #[test]
    fn stale_accounts_are_skipped() {
        let mut s = State::default();
        let mut a = account(10.0, RESET);
        a.stale = true;
        check_usage(&mut s, &snap(vec![a]), &Notifications::default(), "en", at(NOW));
        assert_eq!(s, State::default());
    }

    #[test]
    fn state_round_trips() {
        let dir = crate::test_support::TempDir::new();
        let path = dir.0.join("alerts.json");
        assert_eq!(load_from(&path), State::default());
        let mut s = State::default();
        check_usage(&mut s, &snap(vec![account(91.0, RESET)]), &Notifications::default(), "en", at(NOW));
        s.unexpected.insert("codex:b".into(), at(NOW));
        save_to(&path, &s).unwrap();
        assert_eq!(load_from(&path), s);
    }

    #[test]
    fn plan_wording() {
        let now = at(NOW);
        let due = vec![
            json!({ "kind": "renewal_soon", "provider": "codex", "id": "x", "email": "a@x", "alias": "Work", "at": "2026-10-23T08:00:00Z", "monthly_cost": 200.0 }),
            json!({ "kind": "renewal_wasteful", "provider": "claude", "id": "y", "email": "b@x", "alias": null, "at": "2026-10-25T10:00:00Z", "verdict": "unused", "monthly_cost": 100.0 }),
            json!({ "kind": "runs_out", "provider": "claude", "id": "y", "email": "b@x", "at": "2026-10-22T13:00:00Z", "resets_at": "2026-10-23T12:30:00Z" }),
            json!({ "kind": "extra_usage", "provider": "claude", "id": "y", "email": "b@x", "at": NOW, "percent": 75.0, "used": 300.0, "limit": 400.0 }),
            json!({ "kind": "pro_allowance_cut", "provider": "codex", "id": "x", "at": "2026-11-30T00:00:00Z", "accounts": ["Work", "c@x"] }),
        ];
        let out = plan_notices("en", &due, now);
        let get = |i: usize| match &out[i] { Notice::Plain { title, body, page, phone } => (title.clone(), body.clone(), *page, phone.clone()), _ => panic!() };
        let (title, body, page, phone) = get(0);
        assert_eq!((title.as_str(), body.as_str(), page), ("Codex plan renews in 22h", "Work · $200/month", Some("subscriptions")));
        assert_eq!(phone.unwrap().title, "Codex plan renews tomorrow");
        let (title, body, _, phone) = get(1);
        assert_eq!(title, "Claude plan renews in 3d");
        assert_eq!(body, "b@x · Unused · $100/month\nNot needed? Cancel it on the provider's billing page.");
        assert!(phone.is_none());
        let (title, body, ..) = get(2);
        assert_eq!(title, format!("Claude runs out ~{}", fmt::day_time("en", at("2026-10-22T13:00:00Z"))));
        assert_eq!(body, "Weekly limit · b@x · resets in 1d 2h");
        let (title, body, ..) = get(3);
        assert_eq!((title.as_str(), body.as_str()), ("Claude extra usage at 75%", "b@x · $300 of $400 this month"));
        let (title, body, ..) = get(4);
        assert_eq!((title.as_str(), body.as_str()), ("Codex Pro 200 allowance drops after Nov 30", "At recent use these would hit the weekly limit: Work, c@x"));
    }

    #[test]
    fn digest_wording() {
        let d = json!({
            "week_of": "2026-10-19", "monthly_total": 1300.0, "unpriced": 1, "possible_savings": 200.0,
            "renewing": [{ "provider": "claude", "name": "Work", "at": "2026-10-23T00:00:00Z", "source": "entered", "monthly_cost": 200.0 }],
            "ran_out": ["Work", "Home"],
            "pools": [{ "provider": "codex", "unit": "Pro", "plans_held": 2.0, "plans_needed": 1 }],
        });
        let Notice::Plain { title, body, page, .. } = digest_notice("en", &d) else { panic!() };
        assert_eq!(title, "Your plans this week");
        assert_eq!(page, Some("subscriptions"));
        assert_eq!(body, "$1,300/month on plans · 1 without a price · could save $200\nRenewing: Work (Fri)\nRan out last week: Work, Home\nCodex: usage fits in 1 × Pro; you hold 2");
    }

    #[test]
    fn capacity_and_switch_wording() {
        let now = at(NOW);
        let due = json!({
            "burn": [{ "provider": "codex", "id": "x", "email": "a@x", "alias": null, "trigger": "local_rate", "detected_at": NOW,
                "limit_at": null, "this_pc": { "repo": "slopbar", "model": "gpt-5", "tokens": 9000000, "baseline": 3000000.0 } }],
            "credits": [
                { "kind": "expiring", "provider": "codex", "id": "x", "email": "a@x", "reset_kind": "full", "count": 2, "expires_at": "2026-10-24T10:00:00Z" },
                { "kind": "advice", "provider": "codex", "id": "x", "email": "a@x", "choice": "wait", "reason": "resets_soon", "resets_at": "2026-10-22T11:30:00Z" },
            ],
        });
        let out = capacity_notices("en", &due, now);
        assert_eq!(titles(&out)[0], "Codex burning fast · a@x");
        let Notice::Plain { body, page, .. } = &out[0] else { panic!() };
        assert_eq!(body, "3× its usual tokens on this PC · Lasts until the reset at this pace\nThis PC: slopbar · gpt-5");
        assert_eq!(*page, Some("usage"));
        let Notice::Plain { body, .. } = &out[1] else { panic!() };
        assert_eq!(body, "a@x · 2 unused");
        let Notice::Plain { title, body, .. } = &out[2] else { panic!() };
        assert_eq!((title.as_str(), body.as_str()), ("Codex limit · a@x", "The limit is back in 1h 30m: keep the reset credit."));
        let switch = json!({ "choice": "switch", "switch_to": { "id": "z", "alias": "Spare", "left_percent": 62.4 } });
        assert_eq!(credit_advice_text("en", &switch, now), "Switch to Spare (62% left) and keep the reset credit.");

        let prefs = Notifications::default();
        assert!(switch_notice("en", Provider::Claude, Some("Work"), false, &prefs).is_none());
        assert_eq!(switch_notice("en", Provider::Claude, Some("Work"), true, &prefs),
            Some(plain("Claude".into(), "Claude limit reached — switched to Work".into())));
        let Notice::Plain { title, .. } = monitor_auto_notice("en", None) else { panic!() };
        assert_eq!(title, "Finish connecting monitoring for Unknown account");
    }
}
