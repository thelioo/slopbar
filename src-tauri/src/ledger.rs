//! What the plans have cost, month by month: each paid plan's charge, recorded when its renewal
//! date passes (the cost, tier and date source at that time), plus Claude extra usage spent in
//! each month. Kept in `spend-ledger.json`, keyed like `usage-history.json` (provider and slot id,
//! never an email), so it survives restarts and a renewal that passed while SlopBar was closed is
//! still recorded on the next poll.
//!
//! Amounts are the plan's monthly cost as the report knows it: the US list price unless the user
//! entered what they pay. A tax rate and currency label from settings only change how totals are
//! shown, never what is stored.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::subscriptions::{DateSource, PlanKind, Report};

pub(crate) const VERSION: u8 = 1;
const MAX_FILE: u64 = 4 << 20;
/// Months kept: two years, enough for a trailing twelve months and last year's year-to-date.
const KEEP_MONTHS: usize = 24;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ledger {
    pub version: u8,
    /// By month ("2026-10").
    pub months: BTreeMap<String, Month>,
    /// Per account: the renewal expected next, charged once it passes.
    pub pending: HashMap<String, Pending>,
    /// Per Claude account: the extra-usage amount last read, so each rise counts once.
    pub extra_marks: HashMap<String, f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Month {
    pub charges: Vec<Charge>,
    /// Claude extra usage spent this month, per account, in dollars.
    pub extra: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Charge {
    pub account: String,
    pub at: DateTime<Utc>,
    /// None when the plan's cost was not known.
    pub amount: Option<f64>,
    pub tier: Option<String>,
    pub source: DateSource,
    /// The amount came from the user rather than the list price.
    #[serde(default)]
    pub cost_entered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub at: DateTime<Utc>,
    pub amount: Option<f64>,
    pub tier: Option<String>,
    pub source: DateSource,
    #[serde(default)]
    pub cost_entered: bool,
    /// The user is letting the plan lapse after this day: a renewal after it is not charged.
    #[serde(default)]
    pub until: Option<NaiveDate>,
}

pub fn load() -> Ledger {
    crate::settings::app_dir().map(|d| load_from(&d.join("spend-ledger.json"))).unwrap_or_default()
}

fn load_from(path: &std::path::Path) -> Ledger {
    let fresh = || Ledger { version: VERSION, ..Default::default() };
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_FILE) { return fresh(); }
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Ledger>(&b).ok())
        .filter(|l| l.version == VERSION).unwrap_or_else(fresh)
}

pub fn save(ledger: &Ledger) -> Result<(), String> {
    let dir = crate::settings::app_dir().ok_or("ledger_storage")?;
    save_to(&dir.join("spend-ledger.json"), ledger)
}

fn save_to(path: &std::path::Path, ledger: &Ledger) -> Result<(), String> {
    let bytes = serde_json::to_vec(ledger).map_err(|_| "ledger_storage")?;
    crate::private_store::write(path, &bytes).map_err(|_| "ledger_storage".into())
}

pub fn month_key(t: DateTime<Utc>) -> String { t.format("%Y-%m").to_string() }

fn key(provider: crate::accounts::Provider, id: &str) -> String { format!("{}:{id}", provider.name()) }

/// Folds one report into the ledger: charges every expected renewal that has passed, then notes
/// each paid plan's next one, and adds new Claude extra-usage spend to this month. Returns whether
/// anything changed (and needs saving).
pub fn record(ledger: &mut Ledger, report: &Report, now: DateTime<Utc>) -> bool {
    let before = (ledger.months.clone(), ledger.pending.clone(), ledger.extra_marks.clone());
    ledger.version = VERSION;
    // A renewal that passed, even while SlopBar was closed or the account unreadable.
    let due: Vec<String> = ledger.pending.iter().filter(|(_, p)| p.at <= now).map(|(k, _)| k.clone()).collect();
    for account in due {
        let Some(p) = ledger.pending.remove(&account) else { continue };
        if p.until.is_some_and(|d| p.at.date_naive() > d) { continue; }
        let month = ledger.months.entry(month_key(p.at)).or_default();
        if month.charges.iter().any(|c| c.account == account && c.at.date_naive() == p.at.date_naive()) { continue; }
        // What the plan costs as it renews: the report now, when it still lists the plan; else
        // what was known when the renewal was noted (SlopBar closed, or the account unreadable).
        let now_known = report.accounts.iter().find(|e| key(e.provider, &e.id) == account && e.plan_kind == PlanKind::Personal && e.monthly_cost.is_some());
        let (amount, tier, cost_entered) = match now_known {
            Some(e) => (e.monthly_cost, e.plan.clone().or(p.tier), e.cost_entered),
            None => (p.amount, p.tier, p.cost_entered),
        };
        month.charges.push(Charge { account, at: p.at, amount, tier, source: p.source, cost_entered });
    }
    for e in &report.accounts {
        let account = key(e.provider, &e.id);
        if e.plan_kind == PlanKind::Personal {
            if e.ends_at.is_some() {
                // Cancelled: it will not charge again.
                ledger.pending.remove(&account);
            } else if let Some(r) = e.renewal.as_ref().filter(|r| r.at > now) {
                ledger.pending.insert(account.clone(), Pending { at: r.at, amount: e.monthly_cost, tier: e.plan.clone(),
                    source: r.source, cost_entered: e.cost_entered, until: e.cancel_after });
            }
        }
        // Extra usage costs money only while turned on; a stale reading is not new spend.
        let Some(used) = e.extra_usage.as_ref().filter(|x| x.active() && !e.stale).and_then(|x| x.used) else { continue };
        let spent = match ledger.extra_marks.get(&account) {
            Some(mark) if used >= *mark => used - mark,
            // Lower than last time: a new monthly cycle began, and all of it is new.
            Some(_) => used,
            // First reading: what was used so far this cycle.
            None => used,
        };
        ledger.extra_marks.insert(account.clone(), used);
        if spent > 0.0 {
            *ledger.months.entry(month_key(now)).or_default().extra.entry(account).or_default() += spent;
        }
    }
    while ledger.months.len() > KEEP_MONTHS {
        let oldest = ledger.months.keys().next().cloned().unwrap();
        ledger.months.remove(&oldest);
    }
    (ledger.months.clone(), ledger.pending.clone(), ledger.extra_marks.clone()) != before
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthTotal {
    pub month: String,
    pub plans: f64,
    pub extra_usage: f64,
    pub total: f64,
    /// Charges whose amount was not known, left out of the totals.
    pub unpriced: usize,
}

/// The ledger as the Subscriptions page and reports show it.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// The last twelve months, oldest first; months with nothing recorded are left out.
    pub months: Vec<MonthTotal>,
    pub year_to_date: f64,
    pub trailing_12_months: f64,
    /// The same with the user's tax rate added, when one is set.
    pub year_to_date_with_tax: Option<f64>,
    pub trailing_12_months_with_tax: Option<f64>,
    pub tax_percent: Option<f64>,
    /// A label for the amounts ("USD", "EUR incl. VAT"); display only.
    pub currency: Option<String>,
    /// The first month anything was recorded.
    pub since: Option<String>,
}

fn total(m: &Month) -> (f64, f64, usize) {
    let plans = m.charges.iter().filter_map(|c| c.amount).sum();
    let extra = m.extra.values().sum();
    (plans, extra, m.charges.iter().filter(|c| c.amount.is_none()).count())
}

pub fn summary(ledger: &Ledger, display: &crate::settings::SpendDisplay, now: DateTime<Utc>) -> Summary {
    let this = month_key(now);
    let first = NaiveDate::from_ymd_opt(now.year(), now.month(), 1).and_then(|d| d.checked_sub_months(Months::new(11)));
    let from = first.map(|d| d.format("%Y-%m").to_string()).unwrap_or_default();
    let year = format!("{}-01", now.year());
    let months: Vec<MonthTotal> = ledger.months.range(from..=this.clone()).map(|(month, m)| {
        let (plans, extra_usage, unpriced) = total(m);
        MonthTotal { month: month.clone(), plans: round(plans), extra_usage: round(extra_usage), total: round(plans + extra_usage), unpriced }
    }).collect();
    let year_to_date = round(ledger.months.range(year..=this.clone()).map(|(_, m)| { let (p, x, _) = total(m); p + x }).sum());
    let trailing_12_months = round(months.iter().map(|m| m.total).sum());
    let tax_percent = display.tax_percent.filter(|t| t.is_finite() && *t > 0.0 && *t <= 100.0);
    let taxed = |v: f64| tax_percent.map(|t| round(v * (1.0 + t / 100.0)));
    Summary {
        year_to_date_with_tax: taxed(year_to_date),
        trailing_12_months_with_tax: taxed(trailing_12_months),
        months, year_to_date, trailing_12_months, tax_percent,
        currency: display.currency.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(|c| c.chars().take(24).collect()),
        since: ledger.months.keys().next().cloned(),
    }
}

fn round(v: f64) -> f64 { (v * 100.0).round() / 100.0 }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountUsage, ExtraUsage, Snapshot};
    use crate::settings::{Settings, SpendDisplay, Subscription};
    use crate::subscriptions::{report, History};
    use crate::test_support::TempDir;
    use chrono::Duration;

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    fn max20(id: &str) -> AccountUsage {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "claude", "email": format!("{id}@example.invalid"), "alias": null, "plan": "max", "org": null,
            "active": false, "sources": [], "windows": [], "credits": null, "resets": [], "error": null, "stale": false,
            "fetched_at": null, "monitor_connected": true, "billing": { "tier": "default_claude_max_20x" } })).unwrap()
    }

    fn settings_with_day(id: &str, day: u8) -> Settings {
        let mut s = Settings::default();
        s.subscriptions.insert(format!("claude:{id}"), Subscription { billing_day: Some(day), ..Default::default() });
        s
    }

    fn poll(ledger: &mut Ledger, accounts: Vec<AccountUsage>, settings: &Settings, now: DateTime<Utc>) -> bool {
        let r = report(&Snapshot { accounts, updated_at: Some(now) }, &History::default(), settings, None, now);
        record(ledger, &r, now)
    }

    #[test]
    fn a_renewal_is_charged_once_when_its_date_passes_even_across_a_restart() {
        let dir = TempDir::new();
        let path = dir.0.join("spend-ledger.json");
        let settings = settings_with_day("a", 5);
        let mut ledger = Ledger::default();
        // Oct 20: the plan renews on Nov 5. Nothing is charged yet.
        assert!(poll(&mut ledger, vec![max20("a")], &settings, at("2026-10-20T12:00:00Z")));
        assert!(ledger.months.is_empty());
        assert_eq!(ledger.pending["Claude:a"].at, at("2026-11-05T00:00:00Z"));
        save_to(&path, &ledger).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains('@'), "no emails in the ledger");
        // SlopBar was closed over the renewal; the next start still charges it, once.
        let mut ledger = load_from(&path);
        let later = at("2026-11-07T09:00:00Z");
        poll(&mut ledger, vec![max20("a")], &settings, later);
        poll(&mut ledger, vec![max20("a")], &settings, later + Duration::hours(1));
        let nov = &ledger.months["2026-11"];
        assert_eq!(nov.charges.len(), 1);
        let c = &nov.charges[0];
        assert_eq!((c.at, c.amount, c.tier.as_deref(), c.source, c.cost_entered), (at("2026-11-05T00:00:00Z"), Some(200.0), Some("Max 20x"), DateSource::Entered, false));
        assert_eq!(ledger.pending["Claude:a"].at, at("2026-12-05T00:00:00Z"), "the next renewal is pending");
        // The cost at the time is kept: a later price change does not rewrite November.
        let mut cheaper = settings.clone();
        cheaper.subscriptions.get_mut("claude:a").unwrap().monthly_cost = Some(180.0);
        poll(&mut ledger, vec![max20("a")], &cheaper, at("2026-12-06T00:00:00Z"));
        assert_eq!(ledger.months["2026-11"].charges[0].amount, Some(200.0));
        assert_eq!(ledger.months["2026-12"].charges[0].amount, Some(180.0));
        assert!(ledger.months["2026-12"].charges[0].cost_entered);
    }

    #[test]
    fn cancelled_lapsing_and_free_plans_are_not_charged() {
        let now = at("2026-10-20T12:00:00Z");
        let mut settings = settings_with_day("a", 5);
        let mut ledger = Ledger::default();
        poll(&mut ledger, vec![max20("a")], &settings, now);
        // The user lets it lapse after Nov 4: the Nov 5 renewal is not charged.
        settings.subscriptions.get_mut("claude:a").unwrap().cancel_after = NaiveDate::from_ymd_opt(2026, 11, 4);
        poll(&mut ledger, vec![max20("a")], &settings, now + Duration::hours(1));
        poll(&mut ledger, vec![max20("a")], &settings, at("2026-11-06T00:00:00Z"));
        assert!(ledger.months.values().all(|m| m.charges.is_empty()));
        // The provider says it is cancelled: the pending renewal is dropped.
        let mut ledger = Ledger::default();
        poll(&mut ledger, vec![max20("a")], &settings_with_day("a", 5), now);
        let mut cancelled = max20("a");
        cancelled.billing.cancels_at = Some(at("2026-11-05T00:00:00Z"));
        poll(&mut ledger, vec![cancelled], &Settings::default(), now + Duration::hours(1));
        assert!(ledger.pending.is_empty());
        let mut free = max20("f");
        free.plan = Some("free".into());
        poll(&mut ledger, vec![free], &settings_with_day("f", 5), now);
        assert!(ledger.pending.is_empty());
    }

    #[test]
    fn extra_usage_adds_each_rise_to_its_month_and_survives_a_new_cycle() {
        let mut ledger = Ledger::default();
        let with = |used: f64, enabled: bool| { let mut a = max20("a"); a.extra_usage = Some(ExtraUsage { enabled: Some(enabled), monthly_limit: Some(100.0), used: Some(used), utilization: None }); a };
        let s = Settings::default();
        poll(&mut ledger, vec![with(10.0, true)], &s, at("2026-10-10T00:00:00Z"));
        poll(&mut ledger, vec![with(25.0, true)], &s, at("2026-10-20T00:00:00Z"));
        poll(&mut ledger, vec![with(25.0, true)], &s, at("2026-10-21T00:00:00Z"));
        assert_eq!(ledger.months["2026-10"].extra["Claude:a"], 25.0);
        // A new cycle starts low: all of it is new spend, in the new month.
        poll(&mut ledger, vec![with(4.0, true)], &s, at("2026-11-02T00:00:00Z"));
        assert_eq!(ledger.months["2026-11"].extra["Claude:a"], 4.0);
        // Turned off or stale: nothing more is counted.
        poll(&mut ledger, vec![with(50.0, false)], &s, at("2026-11-03T00:00:00Z"));
        let mut stale = with(60.0, true);
        stale.stale = true;
        poll(&mut ledger, vec![stale], &s, at("2026-11-04T00:00:00Z"));
        assert_eq!(ledger.months["2026-11"].extra["Claude:a"], 4.0);
    }

    #[test]
    fn totals_cover_the_year_and_the_last_twelve_months_with_tax_for_display_only() {
        let mut ledger = Ledger::default();
        let charge = |at_: &str, amount: Option<f64>| Charge { account: "Claude:a".into(), at: at(at_), amount, tier: None, source: DateSource::Provider, cost_entered: false };
        for (m, a) in [("2025-10", 200.0), ("2025-11", 200.0), ("2026-01", 100.0), ("2026-10", 200.0)] {
            ledger.months.entry(m.into()).or_default().charges.push(charge(&format!("{m}-05T00:00:00Z"), Some(a)));
        }
        ledger.months.get_mut("2026-10").unwrap().charges.push(charge("2026-10-09T00:00:00Z", None));
        ledger.months.get_mut("2026-10").unwrap().extra.insert("Claude:a".into(), 12.5);
        let now = at("2026-10-22T00:00:00Z");
        let s = summary(&ledger, &SpendDisplay { tax_percent: Some(10.0), currency: Some(" EUR ".into()) }, now);
        assert_eq!((s.year_to_date, s.trailing_12_months), (312.5, 512.5));
        assert_eq!((s.year_to_date_with_tax, s.currency.as_deref(), s.since.as_deref()), (Some(343.75), Some("EUR"), Some("2025-10")));
        assert_eq!(s.months.first().unwrap().month, "2025-11", "a year ago today is out of the trailing twelve months");
        let oct = s.months.last().unwrap();
        assert_eq!((oct.plans, oct.extra_usage, oct.total, oct.unpriced), (200.0, 12.5, 212.5, 1));
        let s = summary(&ledger, &SpendDisplay { tax_percent: Some(-5.0), currency: None }, now);
        assert_eq!((s.tax_percent, s.year_to_date_with_tax), (None, None));
    }
}
