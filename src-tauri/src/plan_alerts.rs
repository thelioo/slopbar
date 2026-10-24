//! Which plan notifications are due: renewals of plans that look unneeded, any paid plan renewing
//! within a day, the account in use running out of its week before it resets, and Codex Pro 200
//! accounts whose use would not fit the plan's smaller allowance after the cut. Each is shown
//! once (per account and renewal date, or per weekly window), remembered across restarts in a small
//! file. What the notification says is up to the frontend, which owns the translations.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::model::Snapshot;
use crate::settings::Notifications;
use crate::subscriptions::{DateSource, Report, RenewalWarning, Verdict};

const VERSION: u8 = 1;
/// How far ahead of running out the account in use is warned about.
pub const RUNS_OUT_LEAD_HOURS: i64 = 3;
/// What was sent is forgotten after this long: every key names a date that has passed by then.
const FORGET_DAYS: i64 = 60;

/// Notifications already shown, keyed by what they were about, with when.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sent {
    pub version: u8,
    pub sent: HashMap<String, DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A plan that looks unused or underused renews within three days.
    RenewalWasteful,
    /// A paid plan renews within a day.
    RenewalSoon,
    /// The account in use reaches its weekly limit within a few hours, before the reset.
    RunsOut,
    /// Codex Pro 200 accounts that would reach the limit under the allowance after the cut, named
    /// together in `accounts`. Once, before the cut.
    ProAllowanceCut,
    /// A Claude account's extra usage reached 75% or 90% of its monthly limit (`percent`). Once
    /// per account, threshold and month.
    ExtraUsage,
}

/// Shares of the monthly extra-usage limit that are worth a notification.
pub const EXTRA_USAGE_THRESHOLDS: [f64; 2] = [90.0, 75.0];

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub kind: Kind,
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    /// The renewal, or when the limit runs out.
    pub at: DateTime<Utc>,
    /// Where a renewal date comes from.
    pub source: Option<DateSource>,
    pub verdict: Option<Verdict>,
    pub monthly_cost: Option<f64>,
    /// When the weekly window that runs out resets.
    pub resets_at: Option<DateTime<Utc>>,
    /// Every account the notification is about, by alias or email, when there are several.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
    /// Extra usage: the threshold reached, and the amounts in dollars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
}

pub fn load() -> Sent {
    crate::settings::app_dir().map(|d| load_from(&d.join("plan-alerts.json"))).unwrap_or_default()
}

fn load_from(path: &std::path::Path) -> Sent {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Sent>(&b).ok())
        .filter(|s| s.version == VERSION).unwrap_or(Sent { version: VERSION, ..Default::default() })
}

pub fn save(sent: &Sent) -> Result<(), String> {
    let dir = crate::settings::app_dir().ok_or("no config dir")?;
    save_to(&dir.join("plan-alerts.json"), sent)
}

fn save_to(path: &std::path::Path, sent: &Sent) -> Result<(), String> {
    let bytes = serde_json::to_vec(sent).map_err(|e| e.to_string())?;
    crate::private_store::write(path, &bytes).map_err(|e| e.to_string())
}

/// The notifications due now that were not shown before, marking them shown. A notification the
/// user turned off is not marked, so turning it back on still warns about what is pending.
pub fn due(report: &Report, snapshot: &Snapshot, prefs: &Notifications, sent: &mut Sent, now: DateTime<Utc>) -> Vec<Alert> {
    sent.version = VERSION;
    sent.sent.retain(|_, at| now - *at < Duration::days(FORGET_DAYS));
    let mut out = Vec::new();
    let mark = |key: String, sent: &mut Sent| sent.sent.insert(key, now).is_none();
    if prefs.renewals {
        for e in &report.accounts {
            let (Some(warning), Some(renewal)) = (e.renewal_warning, e.renewal.as_ref()) else { continue };
            let account = format!("{}:{}:{}", e.provider.name(), e.id, renewal.at.date_naive());
            let within_day = renewal.at - now <= Duration::hours(24);
            let kind = match warning {
                RenewalWarning::Wasteful if !sent.sent.contains_key(&format!("wasteful:{account}")) => {
                    mark(format!("wasteful:{account}"), sent);
                    // The day-before reminder would only repeat this one.
                    if within_day { mark(format!("soon:{account}"), sent); }
                    Kind::RenewalWasteful
                }
                _ if within_day && mark(format!("soon:{account}"), sent) => Kind::RenewalSoon,
                _ => continue,
            };
            out.push(Alert { kind, provider: e.provider, id: e.id.clone(), email: e.email.clone(), alias: e.alias.clone(),
                at: renewal.at, source: Some(renewal.source), verdict: Some(e.assessment.verdict), monthly_cost: e.monthly_cost, resets_at: None, accounts: vec![], percent: None, used: None, limit: None });
        }
        let cut: Vec<&crate::subscriptions::Entry> = report.accounts.iter()
            .filter(|e| e.allowance_change.as_ref().is_some_and(|c| c.would_hit_limit) && e.ends_at.is_none()).collect();
        if let (Some(first), Some(change)) = (cut.first(), cut.first().and_then(|e| e.allowance_change.as_ref())) {
            if mark(format!("pro_allowance_cut:{}", change.last_day), sent) {
                let at = change.last_day.and_hms_opt(23, 59, 59).map(|d| d.and_utc()).unwrap_or(now);
                out.push(Alert { kind: Kind::ProAllowanceCut, provider: first.provider, id: first.id.clone(), email: first.email.clone(), alias: first.alias.clone(),
                    at, source: None, verdict: None, monthly_cost: None, resets_at: None,
                    accounts: cut.iter().map(|e| e.alias.clone().or_else(|| e.email.clone()).unwrap_or_else(|| e.id.clone())).collect(), percent: None, used: None, limit: None });
            }
        }
    }
    if prefs.extra_usage {
        for e in &report.accounts {
            let Some(x) = e.extra_usage.as_ref().filter(|x| x.active() && !e.stale) else { continue };
            let Some(u) = x.utilization else { continue };
            // The highest threshold reached; reaching 90% first marks 75% too, so one notification.
            let Some(&percent) = EXTRA_USAGE_THRESHOLDS.iter().find(|t| u >= **t) else { continue };
            let month = crate::ledger::month_key(now);
            let key = |t: f64| format!("extra_usage:{}:{}:{month}:{t}", e.provider.name(), e.id);
            if !mark(key(percent), sent) { continue; }
            for t in EXTRA_USAGE_THRESHOLDS.iter().filter(|t| **t < percent) { mark(key(*t), sent); }
            out.push(Alert { kind: Kind::ExtraUsage, provider: e.provider, id: e.id.clone(), email: e.email.clone(), alias: e.alias.clone(),
                at: now, source: None, verdict: None, monthly_cost: None, resets_at: None, accounts: vec![],
                percent: Some(percent), used: x.used, limit: x.monthly_limit });
        }
    }
    if prefs.usage {
        for a in snapshot.accounts.iter().filter(|a| a.active && !a.stale && a.error.is_none()) {
            let Some(at) = a.runs_out_at.filter(|t| *t > now && *t - now <= Duration::hours(RUNS_OUT_LEAD_HOURS)) else { continue };
            let Some(resets_at) = a.windows.iter().find(|w| w.kind == "weekly").and_then(|w| w.resets_at) else { continue };
            // Once per weekly window: its reset, to the hour, absorbs jitter between readings.
            if !mark(format!("runs_out:{}:{}:{}", a.provider.name(), a.id, resets_at.timestamp() / 3600), sent) { continue; }
            out.push(Alert { kind: Kind::RunsOut, provider: a.provider, id: a.id.clone(), email: a.email.clone(), alias: a.alias.clone(),
                at, source: None, verdict: None, monthly_cost: None, resets_at: Some(resets_at), accounts: vec![], percent: None, used: None, limit: None });
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{AccountUsage, BillingHints, UsageWindow};
    use crate::settings::{Settings, Subscription};
    use crate::subscriptions::{report, AccountHistory, History, WindowRecord};
    use crate::test_support::TempDir;

    pub(crate) fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    pub(crate) fn account(id: &str, used: f64, resets_at: DateTime<Utc>) -> AccountUsage {
        AccountUsage {
            id: id.into(), provider: Provider::Codex, email: Some(format!("{id}@example.invalid")), alias: None,
            plan: Some("pro".into()), org: None, active: false, sources: vec![],
            windows: vec![UsageWindow { kind: "weekly".into(), window_seconds: Some(7 * 86400), used_percent: used, resets_at: Some(resets_at) }],
            credits: None, resets: vec![], error: None, monitor_error: None, stale: false, fetched_at: None,
            monitor_connected: true, billing: BillingHints::default(), remote: vec![], overlap: vec![], used_outside: false, runs_out_at: None, unexpected: None, cli_rejected: false, extra_usage: None, leased: None,
        }
    }

    /// Four watched weeks peaking at `peak`, ending before `now`.
    pub(crate) fn watched(peak: f64, now: DateTime<Utc>) -> AccountHistory {
        let windows = (1..=4).map(|i| {
            let end = now - Duration::days(7 * i - 6);
            WindowRecord { kind: "weekly".into(), resets_at: end, peak, first_obs: end - Duration::days(7), last_obs: end - Duration::hours(1),
                hours: 120, days: 7, idle: false, local: false }
        }).collect();
        AccountHistory { first_seen: Some(now - Duration::days(30)), windows, ..Default::default() }
    }

    pub(crate) struct World { pub snapshot: Snapshot, pub history: History, pub settings: Settings }

    /// Three paid Codex plans renewing on the 24th: one unused, one busy, one already cancelled.
    pub(crate) fn world(now: DateTime<Utc>) -> World {
        let mut settings = Settings::default();
        let mut history = History::default();
        let mut accounts = vec![];
        for (id, peak) in [("idle", 0.0), ("busy", 80.0), ("gone", 0.0)] {
            accounts.push(account(id, 1.0, now + Duration::days(5)));
            history.accounts.insert(format!("Codex:{id}"), watched(peak, now));
            settings.subscriptions.insert(format!("codex:{id}"), Subscription { next_billing: Some("2026-10-24".parse().unwrap()), ..Default::default() });
        }
        accounts[2].billing.cancels_at = Some(at("2026-10-24T00:00:00Z"));
        accounts[2].billing.will_renew = Some(false);
        World { snapshot: Snapshot { accounts, updated_at: Some(now) }, history, settings }
    }

    /// The renewal and runs-out notifications due; the Pro 200 one has its own test.
    fn run(w: &World, sent: &mut Sent, now: DateTime<Utc>) -> Vec<(Kind, String)> {
        let r = report(&w.snapshot, &w.history, &w.settings, None, now);
        due(&r, &w.snapshot, &w.settings.notifications, sent, now).into_iter()
            .filter(|a| a.kind != Kind::ProAllowanceCut).map(|a| (a.kind, a.id)).collect()
    }

    #[test]
    fn an_unneeded_plan_is_warned_about_once_per_renewal_and_any_plan_the_day_before() {
        let mut sent = Sent::default();
        // Four days out: too early for anything.
        let w = world(at("2026-10-20T00:00:00Z"));
        assert!(run(&w, &mut sent, at("2026-10-20T00:00:00Z")).is_empty());
        // Three days out: only the unused plan, once however often the widget asks.
        let now = at("2026-10-21T09:00:00Z");
        let w = world(now);
        assert_eq!(run(&w, &mut sent, now), [(Kind::RenewalWasteful, "idle".to_string())]);
        assert!(run(&w, &mut sent, now + Duration::hours(1)).is_empty());
        // The day before: every paid plan gets the softer reminder, the cancelled one excepted.
        let now = at("2026-10-23T06:00:00Z");
        let w = world(now);
        assert_eq!(run(&w, &mut sent, now), [(Kind::RenewalSoon, "idle".to_string()), (Kind::RenewalSoon, "busy".to_string())]);
        assert!(run(&w, &mut sent, now).is_empty());
        // Next month's renewal is a new date: warned about again.
        let now = at("2026-11-21T09:00:00Z");
        let mut w = world(now);
        for s in w.settings.subscriptions.values_mut() { s.next_billing = Some("2026-11-24".parse().unwrap()); }
        w.snapshot.accounts[2].billing.cancels_at = Some(at("2026-11-24T00:00:00Z"));
        assert_eq!(run(&w, &mut sent, now), [(Kind::RenewalWasteful, "idle".to_string())]);
    }

    #[test]
    fn a_warning_first_seen_on_the_last_day_is_one_notification_not_two() {
        let mut sent = Sent::default();
        let now = at("2026-10-23T12:00:00Z");
        assert_eq!(run(&world(now), &mut sent, now), [(Kind::RenewalWasteful, "idle".to_string()), (Kind::RenewalSoon, "busy".to_string())]);
        assert!(run(&world(now), &mut sent, now + Duration::hours(2)).is_empty());
    }

    #[test]
    fn turned_off_means_not_shown_and_not_forgotten_and_what_was_sent_survives_a_restart() {
        let now = at("2026-10-21T09:00:00Z");
        let mut w = world(now);
        w.settings.notifications.renewals = false;
        let mut sent = Sent::default();
        assert!(run(&w, &mut sent, now).is_empty());
        w.settings.notifications.renewals = true;
        assert_eq!(run(&w, &mut sent, now).len(), 1, "pending once turned back on");
        let dir = TempDir::new();
        let path = dir.0.join("plan-alerts.json");
        save_to(&path, &sent).unwrap();
        let mut reloaded = load_from(&path);
        assert!(run(&w, &mut reloaded, now + Duration::hours(3)).is_empty());
        assert!(!std::fs::read_to_string(&path).unwrap().contains('@'), "keyed by slot id, never email");
    }

    #[test]
    fn the_account_in_use_is_warned_once_per_week_shortly_before_it_runs_out() {
        let now = at("2026-10-21T12:00:00Z");
        let mut w = world(now);
        w.settings.subscriptions.clear();
        let a = &mut w.snapshot.accounts[1];
        a.active = true;
        a.runs_out_at = Some(now + Duration::hours(5));
        let mut sent = Sent::default();
        assert!(run(&w, &mut sent, now).is_empty(), "five hours ahead is too early");
        w.snapshot.accounts[1].runs_out_at = Some(now + Duration::hours(2));
        assert_eq!(run(&w, &mut sent, now), [(Kind::RunsOut, "busy".to_string())]);
        assert!(run(&w, &mut sent, now + Duration::minutes(30)).is_empty());
        // Not in use, or stale: nothing.
        let mut other = world(now);
        other.settings.subscriptions.clear();
        other.snapshot.accounts[0].runs_out_at = Some(now + Duration::hours(1));
        other.snapshot.accounts[1].active = true;
        other.snapshot.accounts[1].stale = true;
        other.snapshot.accounts[1].runs_out_at = Some(now + Duration::hours(1));
        assert!(run(&other, &mut Sent::default(), now).is_empty());
        w.settings.notifications.usage = false;
        w.snapshot.accounts[1].windows[0].resets_at = Some(now + Duration::days(9));
        assert!(run(&w, &mut sent, now).is_empty(), "usage alerts off");
    }

    #[test]
    fn pro_200_accounts_that_would_not_fit_half_the_allowance_are_named_once_before_the_cut() {
        let now = at("2026-10-21T12:00:00Z");
        let mut w = world(now);
        w.settings.subscriptions.clear();
        // Peaks of 0 and 80 in recent weeks; "gone" is cancelled. Add a Pro 200 at 50% this week
        // (exactly the new limit), one at 40%, a Pro 100 at 90% and an aliased Pro 200 at 70%.
        for (id, plan, used) in [("half", "pro", 50.0), ("light", "pro", 40.0), ("lite", "prolite", 90.0), ("named", "pro", 70.0)] {
            let mut a = account(id, used, now + Duration::days(3));
            a.plan = Some(plan.into());
            a.fetched_at = Some(now);
            w.snapshot.accounts.push(a);
        }
        w.snapshot.accounts[6].alias = Some("Spare".into());
        let r = report(&w.snapshot, &w.history, &w.settings, None, now);
        let change = |id: &str| r.accounts.iter().find(|e| e.id == id).unwrap().allowance_change.clone();
        let busy = change("busy").unwrap();
        assert_eq!((busy.peak_percent, busy.projected_percent, busy.would_hit_limit), (80.0, 160.0, true));
        assert_eq!(busy.last_day.to_string(), "2026-10-29");
        assert!(change("half").unwrap().would_hit_limit, "50% of the old allowance is all of the new one");
        assert!(!change("light").unwrap().would_hit_limit);
        assert!(!change("idle").unwrap().would_hit_limit);
        assert!(change("lite").is_none(), "only Pro 200 changes");
        let mut sent = Sent::default();
        let alerts = due(&r, &w.snapshot, &w.settings.notifications, &mut sent, now);
        let cut: Vec<&Alert> = alerts.iter().filter(|a| a.kind == Kind::ProAllowanceCut).collect();
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].accounts, ["busy@example.invalid", "half@example.invalid", "Spare"]);
        assert!(serde_json::to_string(cut[0]).unwrap().contains("pro_allowance_cut"));
        // Once only, however often the widget asks.
        assert!(due(&r, &w.snapshot, &w.settings.notifications, &mut sent, now + Duration::days(2)).iter().all(|a| a.kind != Kind::ProAllowanceCut));
        // The last day still warns; the day after, nothing does and the report drops the note.
        let last = at("2026-10-29T20:00:00Z");
        let r = report(&w.snapshot, &w.history, &w.settings, None, last);
        assert!(r.accounts.iter().find(|e| e.id == "busy").unwrap().allowance_change.is_some());
        assert_eq!(due(&r, &w.snapshot, &w.settings.notifications, &mut Sent::default(), last).iter().filter(|a| a.kind == Kind::ProAllowanceCut).count(), 1);
        let after = at("2026-10-30T00:30:00Z");
        let r = report(&w.snapshot, &w.history, &w.settings, None, after);
        assert!(r.accounts.iter().all(|e| e.allowance_change.is_none()));
        assert!(due(&r, &w.snapshot, &w.settings.notifications, &mut Sent::default(), after).iter().all(|a| a.kind != Kind::ProAllowanceCut));
        // Plan notifications off: not shown, and still pending.
        w.settings.notifications.renewals = false;
        let r = report(&w.snapshot, &w.history, &w.settings, None, now);
        let mut fresh = Sent::default();
        assert!(due(&r, &w.snapshot, &w.settings.notifications, &mut fresh, now).is_empty());
        assert!(fresh.sent.is_empty());
    }

    #[test]
    fn extra_usage_warns_at_75_and_90_percent_once_a_month_and_only_when_turned_on() {
        let now = at("2026-10-20T12:00:00Z");
        let mut acc = account("claude1", 10.0, now + Duration::days(3));
        acc.provider = Provider::Claude;
        let snapshot = |u: f64, enabled: bool| { let mut a = acc.clone(); a.extra_usage = Some(crate::model::ExtraUsage { enabled: Some(enabled), monthly_limit: Some(400.0), used: Some(u * 4.0), utilization: Some(u) }); Snapshot { accounts: vec![a], updated_at: Some(now) } };
        let mut sent = Sent::default();
        let run = |u: f64, enabled: bool, prefs: &Notifications, sent: &mut Sent, now: DateTime<Utc>| {
            let s = snapshot(u, enabled);
            due(&report(&s, &History::default(), &Settings::default(), None, now), &s, prefs, sent, now)
        };
        let on = Notifications::default();
        assert!(run(70.0, true, &on, &mut sent, now).is_empty());
        let a = run(76.0, true, &on, &mut sent, now);
        assert_eq!((a.len(), a[0].kind, a[0].percent, a[0].used, a[0].limit), (1, Kind::ExtraUsage, Some(75.0), Some(304.0), Some(400.0)));
        assert!(run(80.0, true, &on, &mut sent, now).is_empty(), "once per threshold");
        assert_eq!(run(91.0, true, &on, &mut sent, now)[0].percent, Some(90.0));
        assert!(run(95.0, true, &on, &mut sent, now).is_empty());
        // A new month warns again; jumping straight to 90% is one notification, not two.
        let next = at("2026-11-03T00:00:00Z");
        let a = run(92.0, true, &on, &mut sent, next);
        assert_eq!((a.len(), a[0].percent), (1, Some(90.0)));
        assert!(run(92.0, true, &on, &mut sent, next).is_empty());
        // Turned off in Notifications, or extra usage not enabled: nothing, and nothing marked.
        let mut sent = Sent::default();
        let off = Notifications { extra_usage: false, ..Default::default() };
        assert!(run(95.0, true, &off, &mut sent, now).is_empty());
        assert!(run(95.0, false, &on, &mut sent, now).is_empty());
        assert_eq!(run(95.0, true, &on, &mut sent, now).len(), 1);
    }
}
