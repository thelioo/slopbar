//! The Monday digest: one notification each Monday morning (local time) summing up the plans —
//! monthly spend, what could be saved, plans renewing this week, accounts that ran out last week,
//! and how many plans the usage needs. Shown once per week, remembered with the plan notifications
//! (see `plan_alerts`). What it says is up to the frontend, which owns the translations.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Utc, Weekday};
use serde::Serialize;

use crate::accounts::Provider;
use crate::plan_alerts::Sent;
use crate::subscriptions::{DateSource, History, PlanKind, Report};

/// The digest is due from this local hour on Mondays.
pub const HOUR: u32 = 8;

#[derive(Debug, Clone, Serialize)]
pub struct Digest {
    /// The Monday it is for.
    pub week_of: NaiveDate,
    /// Sum of the paid plans' monthly cost, where known.
    pub monthly_total: f64,
    /// Paid plans whose cost is unknown, left out of `monthly_total`.
    pub unpriced: usize,
    pub possible_savings: f64,
    pub renewing: Vec<Renewing>,
    /// Accounts that reached their weekly limit in the past seven days, by alias or email.
    pub ran_out: Vec<String>,
    pub pools: Vec<PoolLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Renewing {
    pub provider: Provider,
    pub name: String,
    pub at: DateTime<Utc>,
    /// An entered date is a calendar day (midnight UTC); the others are moments.
    pub source: DateSource,
    pub monthly_cost: Option<f64>,
}

/// "Your usage fits in `plans_needed` plans; you hold `plans_held`", in `unit` plans.
#[derive(Debug, Clone, Serialize)]
pub struct PoolLine {
    pub provider: Provider,
    pub unit: &'static str,
    pub plans_held: f64,
    pub plans_needed: u32,
}

/// The Monday a digest is due for at local time `local`: Mondays from `HOUR` on.
pub fn week_due(local: NaiveDateTime) -> Option<NaiveDate> {
    use chrono::Timelike;
    (local.weekday() == Weekday::Mon && local.hour() >= HOUR).then(|| local.date())
}

fn key(week_of: NaiveDate) -> String {
    format!("digest:{week_of}")
}

/// The digest when one is due and was not shown this week, marking it shown. Turned off, nothing
/// is marked, so turning it back on on a Monday still shows that week's. Without paid plans there
/// is nothing to sum up.
pub fn take(report: &Report, history: &History, enabled: bool, sent: &mut Sent, local: NaiveDateTime, now: DateTime<Utc>) -> Option<Digest> {
    let week_of = week_due(local)?;
    if !enabled || sent.sent.contains_key(&key(week_of)) { return None; }
    let digest = build(report, history, week_of, now)?;
    sent.sent.insert(key(week_of), now);
    Some(digest)
}

pub fn build(report: &Report, history: &History, week_of: NaiveDate, now: DateTime<Utc>) -> Option<Digest> {
    let paid: Vec<_> = report.accounts.iter().filter(|e| matches!(e.plan_kind, PlanKind::Personal | PlanKind::Organization) && e.ends_at.is_none()).collect();
    if paid.is_empty() { return None; }
    let name = |e: &crate::subscriptions::Entry| e.alias.clone().or_else(|| e.email.clone()).unwrap_or_else(|| e.id.clone());
    let mut renewing: Vec<Renewing> = paid.iter()
        .filter_map(|e| {
            let (at, source) = e.renewal.as_ref().map(|r| (r.at, r.source))?;
            (at >= now - Duration::days(1) && at < now + Duration::days(7)).then(|| Renewing { provider: e.provider, name: name(e), at, source, monthly_cost: e.monthly_cost })
        })
        .collect();
    renewing.sort_by_key(|r| r.at);
    let ran_out = report.accounts.iter()
        .filter(|e| history.accounts.get(&format!("{}:{}", e.provider.name(), e.id))
            .is_some_and(|h| h.windows.iter().any(|r| r.kind == "weekly" && r.hit_limit() && r.last_obs > now - Duration::days(7))))
        .map(name)
        .collect();
    let pools = report.providers.iter()
        .filter_map(|p| Some(PoolLine { provider: p.provider, unit: p.pool.unit, plans_held: p.pool.plans_held, plans_needed: p.pool.plans_needed? }))
        .collect();
    Some(Digest {
        week_of,
        monthly_total: report.providers.iter().map(|p| p.monthly_total).sum(),
        unpriced: report.providers.iter().map(|p| p.unpriced).sum(),
        possible_savings: report.providers.iter().map(|p| p.possible_savings).sum(),
        renewing,
        ran_out,
        pools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_alerts::tests::{at, world};
    use crate::subscriptions::{report, WindowRecord};

    fn local(s: &str) -> NaiveDateTime { s.parse().unwrap() }

    #[test]
    fn due_on_monday_mornings_only() {
        // 2026-10-19 and 2026-10-26 are Mondays.
        assert_eq!(week_due(local("2026-10-19T07:59:00")), None, "too early");
        assert_eq!(week_due(local("2026-10-19T08:00:00")), "2026-10-19".parse().ok());
        assert_eq!(week_due(local("2026-10-19T23:30:00")), "2026-10-19".parse().ok(), "a PC woken later on Monday still gets it");
        assert_eq!(week_due(local("2026-10-20T09:00:00")), None, "not on Tuesday");
        assert_eq!(week_due(local("2026-10-25T09:00:00")), None, "not on Sunday");
    }

    #[test]
    fn shown_once_per_week_and_held_back_while_off() {
        let now = at("2026-10-19T09:00:00Z");
        let w = world(now);
        let r = report(&w.snapshot, &w.history, &w.settings, None, now);
        let mut sent = Sent::default();
        assert!(take(&r, &w.history, false, &mut sent, local("2026-10-19T09:00:00"), now).is_none());
        assert!(sent.sent.is_empty(), "off means not marked");
        assert!(take(&r, &w.history, true, &mut sent, local("2026-10-19T09:00:00"), now).is_some());
        assert!(take(&r, &w.history, true, &mut sent, local("2026-10-19T15:00:00"), now + Duration::hours(6)).is_none(), "once that Monday");
        assert!(take(&r, &w.history, true, &mut sent, local("2026-10-20T09:00:00"), now + Duration::days(1)).is_none());
        let next = now + Duration::days(7);
        let w = world(next);
        let r = report(&w.snapshot, &w.history, &w.settings, None, next);
        assert_eq!(take(&r, &w.history, true, &mut sent, local("2026-10-26T08:30:00"), next).unwrap().week_of.to_string(), "2026-10-26");
    }

    #[test]
    fn sums_spend_and_names_renewals_and_accounts_that_ran_out() {
        let now = at("2026-10-19T09:00:00Z");
        let mut w = world(now);
        // "busy" hit its limit three days ago.
        w.history.accounts.get_mut("Codex:busy").unwrap().windows.push(WindowRecord { kind: "weekly".into(), resets_at: now + Duration::days(2),
            peak: 100.0, first_obs: now - Duration::days(5), last_obs: now - Duration::days(3), hours: 80, days: 5, idle: false, local: false });
        let r = report(&w.snapshot, &w.history, &w.settings, None, now);
        let d = build(&r, &w.history, "2026-10-19".parse().unwrap(), now).unwrap();
        let expected: f64 = r.providers.iter().map(|p| p.monthly_total).sum();
        assert!(expected > 0.0 && (d.monthly_total - expected).abs() < 1e-9);
        let mut renewing: Vec<_> = d.renewing.iter().map(|x| x.name.as_str()).collect();
        renewing.sort();
        assert_eq!(renewing, ["busy@example.invalid", "idle@example.invalid"], "both plans renewing on the 24th, the cancelled one left out");
        assert_eq!(d.ran_out, ["busy@example.invalid"]);
        assert!(serde_json::to_string(&d).unwrap().contains("\"week_of\":\"2026-10-19\""));
    }

    #[test]
    fn nothing_to_say_without_paid_plans() {
        let now = at("2026-10-19T09:00:00Z");
        let mut w = world(now);
        for a in &mut w.snapshot.accounts { a.plan = Some("free".into()); }
        let r = report(&w.snapshot, &w.history, &w.settings, None, now);
        let mut sent = Sent::default();
        assert!(take(&r, &w.history, true, &mut sent, local("2026-10-19T09:00:00"), now).is_none());
        assert!(sent.sent.is_empty(), "a later refresh with paid plans can still show it");
    }
}
