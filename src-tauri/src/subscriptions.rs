//! Whether each paid plan earns its keep: a small history of every completed weekly window per
//! account (how far it got before resetting, and how much of it we actually saw), a verdict from
//! that history, and the next renewal with where that date came from.
//!
//! Nothing here guesses usage. A week we did not watch is missing, never 0%: an account is only
//! called unused after several weeks of real readings near zero. SlopBar cannot cancel a plan;
//! the report only says which ones look unneeded and links to the provider's own billing page.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Months, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::model::{AccountUsage, Snapshot};
use crate::settings::Settings;

const VERSION: u8 = 1;
/// Completed weeks kept per account and window kind (about three months).
const KEEP: usize = 12;
/// The verdict looks at this many of the latest completed weeks.
const RECENT: usize = 4;
/// A week counts toward a verdict only when we watched it this long, on this many days.
const MIN_HOURS: u16 = 24;
const MIN_DAYS: u8 = 3;
const LIMIT_PERCENT: f64 = 99.5;
const UNUSED_PERCENT: f64 = 5.0;
const UNDERUSED_PERCENT: f64 = 25.0;
const WEEK_SECONDS: i64 = 7 * 86400;
/// Two readings of one window can report a reset a little apart.
const SAME_WINDOW_SECONDS: i64 = 3600;
const MAX_FILE: u64 = 4 << 20;
/// An unused window's reset sits this close to a full week ahead.
const IDLE_SLACK_SECONDS: i64 = 15 * 60;
const FORGET_DAYS: i64 = 14;

/// Weekly windows the history records. Session windows say nothing about a plan's value.
const KINDS: [&str; 3] = ["weekly", "weekly_opus", "weekly_sonnet"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub version: u8,
    /// Keyed by `monitoring::key` (provider and slot id), never by email.
    pub accounts: HashMap<String, AccountHistory>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountHistory {
    pub first_seen: Option<DateTime<Utc>>,
    /// The last poll that listed this account, readable or not.
    pub last_seen: Option<DateTime<Utc>>,
    pub windows: Vec<WindowRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowRecord {
    pub kind: String,
    /// When this window resets: its identity.
    pub resets_at: DateTime<Utc>,
    /// The highest share used among fresh readings.
    pub peak: f64,
    pub first_obs: DateTime<Utc>,
    pub last_obs: DateTime<Utc>,
    /// Distinct clock hours and calendar days (UTC) with a fresh reading.
    pub hours: u16,
    pub days: u8,
    /// An idle stretch rather than a provider window: an unused week has no fixed reset (Codex
    /// reports one a full week ahead on every poll, Claude none), so it is cut into weeks from
    /// when SlopBar started watching it, and ends early when usage begins.
    #[serde(default)]
    pub idle: bool,
}

impl WindowRecord {
    fn hit_limit(&self) -> bool { self.peak >= LIMIT_PERCENT }
    fn counts(&self) -> bool { self.hours >= MIN_HOURS && self.days >= MIN_DAYS }
}

pub fn load() -> History {
    crate::settings::app_dir().map(|d| load_from(&d.join("usage-history.json"))).unwrap_or_default()
}

fn load_from(path: &std::path::Path) -> History {
    let fresh = || History { version: VERSION, ..Default::default() };
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_FILE) { return fresh(); }
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<History>(&b).ok())
        .filter(|h| h.version == VERSION).unwrap_or_else(fresh)
}

pub fn save(history: &History) -> Result<(), String> {
    let dir = crate::settings::app_dir().ok_or("history_storage")?;
    save_to(&dir.join("usage-history.json"), history)
}

fn save_to(path: &std::path::Path, history: &History) -> Result<(), String> {
    let bytes = serde_json::to_vec(history).map_err(|_| "history_storage")?;
    crate::private_store::write(path, &bytes).map_err(|_| "history_storage".into())
}

/// Folds one poll's reading into the account's history. Only fresh, successful readings count:
/// a stale or failed one, or a reset that passed unseen, leaves the history as it was.
pub fn observe(h: &mut AccountHistory, acc: &AccountUsage, now: DateTime<Utc>) {
    if acc.stale || acc.error.is_some() { return; }
    h.first_seen.get_or_insert(now);
    for w in &acc.windows {
        if !KINDS.contains(&w.kind.as_str()) { continue; }
        // Codex says how long a window is; only a true week is a weekly record.
        if w.window_seconds.is_some_and(|s| s != WEEK_SECONDS) { continue; }
        let week_ahead = w.resets_at.map_or(true, |r| (r - now).num_seconds() > WEEK_SECONDS - IDLE_SLACK_SECONDS);
        if w.used_percent <= 0.0 && week_ahead {
            observe_idle(h, &w.kind, now);
            continue;
        }
        let Some(resets_at) = w.resets_at else { continue };
        // A window whose reset has passed belongs to the next poll's numbers, not this one.
        if resets_at <= now { continue; }
        // Usage began: the idle stretch before it ends here.
        if let Some(r) = h.windows.iter_mut().find(|r| r.kind == w.kind && r.idle && r.resets_at > now) {
            r.resets_at = now;
        }
        let current = h.windows.iter_mut().rev().find(|r| r.kind == w.kind && !r.idle
            && (r.resets_at - resets_at).num_seconds().abs() < SAME_WINDOW_SECONDS);
        match current {
            Some(r) => {
                seen(r, now);
                r.peak = r.peak.max(w.used_percent);
            }
            None => push(h, WindowRecord { kind: w.kind.clone(), resets_at, peak: w.used_percent,
                first_obs: now, last_obs: now, hours: 1, days: 1, idle: false }, now),
        }
    }
}

fn observe_idle(h: &mut AccountHistory, kind: &str, now: DateTime<Utc>) {
    match h.windows.iter_mut().find(|r| r.kind == kind && r.resets_at > now) {
        // An open window already holds this week: a 0% reading leaves its peak alone.
        Some(r) => seen(r, now),
        None => push(h, WindowRecord { kind: kind.into(), resets_at: now + chrono::Duration::seconds(WEEK_SECONDS),
            peak: 0.0, first_obs: now, last_obs: now, hours: 1, days: 1, idle: true }, now),
    }
}

fn seen(r: &mut WindowRecord, now: DateTime<Utc>) {
    if now.format("%Y%m%d%H").to_string() != r.last_obs.format("%Y%m%d%H").to_string() { r.hours = r.hours.saturating_add(1); }
    if now.date_naive() != r.last_obs.date_naive() { r.days = r.days.saturating_add(1); }
    r.last_obs = r.last_obs.max(now);
}

/// Adds a record, then drops the oldest completed ones of its kind beyond `KEEP`. A window still
/// open is never dropped.
fn push(h: &mut AccountHistory, record: WindowRecord, now: DateTime<Utc>) {
    let kind = record.kind.clone();
    h.windows.push(record);
    loop {
        let done: Vec<usize> = h.windows.iter().enumerate().filter(|(_, r)| r.kind == kind && r.resets_at <= now).map(|(i, _)| i).collect();
        let total = h.windows.iter().filter(|r| r.kind == kind).count();
        if total <= KEEP || done.is_empty() { break; }
        let oldest = *done.iter().min_by_key(|i| h.windows[**i].resets_at).unwrap();
        h.windows.remove(oldest);
    }
}

/// Records every account of a snapshot. An account that is no longer saved is forgotten only
/// after `FORGET_DAYS` unseen: one unreadable login file must not erase weeks of history.
pub fn record(history: &mut History, snapshot: &Snapshot, valid: &std::collections::HashSet<String>, now: DateTime<Utc>) {
    history.version = VERSION;
    for acc in &snapshot.accounts {
        let h = history.accounts.entry(history_key(acc.provider, &acc.id)).or_default();
        h.last_seen = Some(now);
        observe(h, acc, now);
    }
    let cutoff = now - chrono::Duration::days(FORGET_DAYS);
    history.accounts.retain(|k, h| valid.contains(k) || h.last_seen.is_some_and(|t| t > cutoff));
}

/// Same as `monitoring::key`, from a reading rather than a slot.
fn history_key(provider: Provider, id: &str) -> String {
    format!("{}:{id}", provider.name())
}

/// Settings keys per account, as aliases use (`providers::alias_key`).
fn pref_key(provider: Provider, id: &str) -> String {
    crate::providers::alias_key(provider, id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Fewer than two weeks we watched closely enough to judge.
    InsufficientData,
    /// Ran into its weekly limit in most recent weeks.
    ExhaustedWeekly,
    Healthy,
    /// Mostly idle: a smaller plan might do.
    Underused,
    /// Weeks of readings near zero.
    Unused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence { Low, High }

#[derive(Debug, Clone, Serialize)]
pub struct Assessment {
    pub verdict: Verdict,
    pub confidence: Confidence,
    /// Recent completed weeks watched closely enough to count, out of `RECENT`.
    pub weeks_counted: u8,
    pub weeks_needed: u8,
    /// Average and median of those weeks' highest usage, in percent.
    pub average_peak: Option<f64>,
    pub median_peak: Option<f64>,
    /// How many of them reached the limit.
    pub limit_weeks: u8,
    /// A model-specific weekly window that reached its limit ("weekly_opus").
    pub sub_window_limited: Option<String>,
    /// Paying for another period looks unneeded on this evidence.
    pub renew_wasteful: bool,
}

pub fn assess(h: &AccountHistory, now: DateTime<Utc>) -> Assessment {
    let recent = |kind: &str| -> Vec<&WindowRecord> {
        let mut done: Vec<&WindowRecord> = h.windows.iter().filter(|r| r.kind == kind && r.resets_at <= now && r.counts()).collect();
        done.sort_by_key(|r| std::cmp::Reverse(r.resets_at));
        done.truncate(RECENT);
        done
    };
    let weeks = recent("weekly");
    let counted = weeks.len() as u8;
    let peaks: Vec<f64> = weeks.iter().map(|r| r.peak).collect();
    let average_peak = (!peaks.is_empty()).then(|| peaks.iter().sum::<f64>() / peaks.len() as f64);
    let median_peak = median(&peaks);
    // A model-specific cap that keeps running out binds just like the overall one.
    let sub_window_limited = ["weekly_opus", "weekly_sonnet"].into_iter()
        .find(|k| recent(k).iter().filter(|r| r.hit_limit()).count() >= 2).map(String::from);
    let limit_weeks = weeks.iter().filter(|r| r.hit_limit()).count() as u8;
    let confidence = if counted as usize >= RECENT && weeks.iter().map(|r| r.hours as u32).sum::<u32>() >= 72 * RECENT as u32 {
        Confidence::High
    } else {
        Confidence::Low
    };
    let verdict = if counted < 2 {
        Verdict::InsufficientData
    } else if limit_weeks >= 2 || limit_weeks * 2 >= counted && limit_weeks > 0 || sub_window_limited.is_some() {
        Verdict::ExhaustedWeekly
    } else if counted >= 3 && peaks.iter().all(|p| *p <= UNUSED_PERCENT) {
        Verdict::Unused
    } else if median_peak.is_some_and(|m| m <= UNDERUSED_PERCENT) {
        Verdict::Underused
    } else {
        Verdict::Healthy
    };
    let renew_wasteful = verdict == Verdict::Unused || verdict == Verdict::Underused && confidence == Confidence::High;
    Assessment { verdict, confidence, weeks_counted: counted, weeks_needed: 2, average_peak, median_peak,
        limit_weeks, sub_window_limited, renew_wasteful }
}

fn median(values: &[f64]) -> Option<f64> {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]) / 2.0),
    }
}

/// Where a renewal date comes from, so the window never presents a guess as a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSource {
    /// The user typed the billing day.
    Entered,
    /// The provider's own subscription end, current as of its last check.
    Provider,
    /// Counted forward from when the subscription started or last ended.
    Estimated,
}

#[derive(Debug, Clone, Serialize)]
pub struct Renewal {
    pub at: DateTime<Utc>,
    pub source: DateSource,
}

/// What kind of plan this is, for what the window offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    Free,
    /// Paid by the user: they can cancel it themselves.
    Personal,
    /// Paid by an organization (team, business, enterprise, edu).
    Organization,
    Unknown,
}

pub fn plan_kind(plan: Option<&str>) -> PlanKind {
    match plan.map(str::to_ascii_lowercase).as_deref() {
        None | Some("") => PlanKind::Unknown,
        Some("free") => PlanKind::Free,
        Some("pro" | "max" | "plus" | "go") => PlanKind::Personal,
        Some("team" | "enterprise" | "business" | "edu" | "education") => PlanKind::Organization,
        Some(_) => PlanKind::Unknown,
    }
}

/// The plan's name as the window shows it, including Claude Max's size.
pub fn plan_label(acc: &AccountUsage) -> Option<String> {
    let plan = acc.plan.as_deref()?;
    let tier = acc.billing.tier.as_deref().unwrap_or("");
    Some(match (acc.provider, plan) {
        (Provider::Claude, "max") if tier.ends_with("_20x") => "Max 20x".into(),
        (Provider::Claude, "max") if tier.ends_with("_5x") => "Max 5x".into(),
        (_, p) => { let mut c = p.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default() }
    })
}

/// US list price per month, when the plan name alone settles it. ChatGPT Pro comes in more than
/// one size and the token does not say which, so it has none.
pub fn list_price(acc: &AccountUsage) -> Option<f64> {
    Some(match (acc.provider, plan_label(acc)?.as_str()) {
        (Provider::Claude, "Pro") => 20.0,
        (Provider::Claude, "Max 5x") => 100.0,
        (Provider::Claude, "Max 20x") => 200.0,
        (Provider::Codex, "Plus") => 20.0,
        _ => return None,
    })
}

/// The next renewal after `now`: the user's billing day first, then the provider's own next
/// charge (Codex's account record), then the end of the paid period while it is still ahead,
/// then a monthly count from the last known date. A plan set to cancel has no renewal.
pub fn next_renewal(acc: &AccountUsage, prefs: &crate::settings::Subscription, now: DateTime<Utc>) -> Option<Renewal> {
    if ends_at(acc, now).is_some() { return None; }
    if let Some(date) = prefs.next_billing {
        // A corrected date stays right for later periods too: count on from it.
        let step = Months::new(if matches!(acc.billing.billing_period.as_deref(), Some("yearly" | "annual" | "year")) { 12 } else { 1 });
        let mut d = date;
        while d < now.date_naive() { d = d.checked_add_months(step)?; }
        return Some(Renewal { at: Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0)?), source: DateSource::Entered });
    }
    if let Some(day) = prefs.billing_day.filter(|d| (1..=31).contains(d)) {
        return Some(Renewal { at: next_day_of_month(day, now)?, source: DateSource::Entered });
    }
    let b = &acc.billing;
    if let Some(at) = b.renews_at.filter(|t| *t > now) {
        return Some(Renewal { at, source: DateSource::Provider });
    }
    if let Some(until) = b.active_until.filter(|t| *t > now) {
        return Some(Renewal { at: until, source: DateSource::Provider });
    }
    let anchor = b.renews_at.or(b.active_until).or(b.started_at)?;
    let mut at = anchor;
    for months in 1..=240u32 {
        if at > now { break; }
        at = anchor.checked_add_months(Months::new(months))?;
    }
    (at > now).then_some(Renewal { at, source: DateSource::Estimated })
}

/// When a plan the provider says will not renew stops being paid for.
pub fn ends_at(acc: &AccountUsage, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let b = &acc.billing;
    let ending = b.cancels_at.is_some() || b.will_renew == Some(false);
    ending.then(|| b.cancels_at.or(b.active_until).or(b.renews_at)).flatten().filter(|t| *t > now)
}

fn next_day_of_month(day: u8, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let on = |y: i32, m: u32| -> Option<DateTime<Utc>> {
        // A billing day past the month's end falls on its last day, as card billing does.
        let last = (28..=31u32).rev().find(|d| NaiveDate::from_ymd_opt(y, m, *d).is_some())?;
        let date = NaiveDate::from_ymd_opt(y, m, (day as u32).min(last))?;
        Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?))
    };
    let this = on(now.year(), now.month())?;
    if this > now { return Some(this); }
    let next = now.date_naive().with_day(1)?.checked_add_months(Months::new(1))?;
    on(next.year(), next.month())
}

/// The week in progress, from the live reading: how far it got and how much of it remains.
#[derive(Debug, Clone, Serialize)]
pub struct ThisWeek {
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    /// Usage so far divided by the share of the week that has passed: above 100 runs out early.
    pub pace_percent: Option<f64>,
}

fn this_week(acc: &AccountUsage, now: DateTime<Utc>) -> Option<ThisWeek> {
    let w = acc.windows.iter().find(|w| w.kind == "weekly")?;
    let elapsed = w.resets_at.map(|r| WEEK_SECONDS - (r - now).num_seconds()).filter(|e| *e > 6 * 3600 && *e <= WEEK_SECONDS);
    Some(ThisWeek {
        used_percent: w.used_percent,
        resets_at: w.resets_at,
        pace_percent: if acc.stale { None } else { elapsed.map(|e| w.used_percent * WEEK_SECONDS as f64 / e as f64) },
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub plan: Option<String>,
    pub plan_kind: PlanKind,
    pub active: bool,
    pub stale: bool,
    pub this_week: Option<ThisWeek>,
    pub assessment: Assessment,
    /// Days since SlopBar first saw this account.
    pub days_tracked: i64,
    pub renewal: Option<Renewal>,
    /// The provider's subscription end, and when it last checked it.
    pub provider_until: Option<DateTime<Utc>>,
    pub provider_checked: Option<DateTime<Utc>>,
    /// Bought in an app store: cancelled there, not on the provider's site.
    pub store_billed: bool,
    pub monthly_cost: Option<f64>,
    /// The cost came from the user rather than the list price.
    pub cost_entered: bool,
    pub cancel_after: Option<NaiveDate>,
    /// The provider says this plan is cancelled and ends then: it will not charge again.
    pub ends_at: Option<DateTime<Utc>>,
    /// A payment failed and the provider is waiting for it.
    pub delinquent: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummary {
    pub provider: Provider,
    pub paid: usize,
    pub exhausted: usize,
    pub unused: usize,
    /// Monthly cost of the paid plans whose cost is known.
    pub monthly_total: f64,
    /// Plans whose cost is not known, so the total leaves them out.
    pub unpriced: usize,
    /// Monthly cost of the plans whose renewal looks unneeded.
    pub possible_savings: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub accounts: Vec<Entry>,
    pub providers: Vec<ProviderSummary>,
    pub generated_at: DateTime<Utc>,
}

pub fn report(snapshot: &Snapshot, history: &History, settings: &Settings, now: DateTime<Utc>) -> Report {
    let empty = AccountHistory::default();
    let accounts: Vec<Entry> = snapshot.accounts.iter().map(|acc| {
        let prefs = settings.subscriptions.get(&pref_key(acc.provider, &acc.id)).cloned().unwrap_or_default();
        let h = history.accounts.get(&history_key(acc.provider, &acc.id)).unwrap_or(&empty);
        let kind = plan_kind(acc.plan.as_deref());
        let paid = kind == PlanKind::Personal;
        let monthly_cost = prefs.monthly_cost.or_else(|| list_price(acc).filter(|_| paid));
        Entry {
            provider: acc.provider,
            id: acc.id.clone(),
            email: acc.email.clone(),
            alias: acc.alias.clone(),
            plan: plan_label(acc),
            plan_kind: kind,
            active: acc.active,
            stale: acc.stale,
            this_week: this_week(acc, now),
            assessment: assess(h, now),
            days_tracked: h.first_seen.map_or(0, |t| (now - t).num_days().max(0)),
            renewal: if paid { next_renewal(acc, &prefs, now) } else { None },
            provider_until: acc.billing.active_until,
            provider_checked: acc.billing.checked_at,
            store_billed: acc.billing.billing_type.as_deref().is_some_and(|t| t.contains("apple") || t.contains("google") || t.contains("app_store") || t.contains("play")),
            cost_entered: prefs.monthly_cost.is_some(),
            monthly_cost,
            cancel_after: prefs.cancel_after,
            ends_at: if paid { ends_at(acc, now) } else { None },
            delinquent: acc.billing.delinquent == Some(true),
        }
    }).collect();
    let providers = Provider::ALL.into_iter().filter(|p| accounts.iter().any(|a| a.provider == *p)).map(|provider| {
        let paid: Vec<&Entry> = accounts.iter().filter(|a| a.provider == provider && a.plan_kind == PlanKind::Personal).collect();
        ProviderSummary {
            provider,
            paid: paid.len(),
            exhausted: paid.iter().filter(|a| a.assessment.verdict == Verdict::ExhaustedWeekly).count(),
            unused: paid.iter().filter(|a| a.assessment.verdict == Verdict::Unused).count(),
            monthly_total: paid.iter().filter_map(|a| a.monthly_cost).sum(),
            unpriced: paid.iter().filter(|a| a.monthly_cost.is_none()).count(),
            // A plan already cancelled saves nothing more.
            possible_savings: paid.iter().filter(|a| a.assessment.renew_wasteful && a.ends_at.is_none()).filter_map(|a| a.monthly_cost).sum(),
        }
    }).collect();
    Report { accounts, providers, generated_at: now }
}

/// The provider pages where a plan is managed or cancelled. Fixed here, so nothing the window or
/// an agent passes can choose where the browser goes.
pub fn billing_page(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "https://claude.ai/settings/billing",
        Provider::Codex => "https://chatgpt.com/#settings/Account",
    }
}

/// Accounts whose plan the user is letting lapse, past its last paid day: automatic switching
/// must not move onto them.
pub fn lapsed(settings: &Settings, provider: Provider, id: &str, now: DateTime<Utc>) -> bool {
    settings.subscriptions.get(&pref_key(provider, id)).and_then(|p| p.cancel_after).is_some_and(|d| d < now.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BillingHints, UsageWindow};
    use crate::test_support::TempDir;
    use chrono::Duration;

    fn days(n: i64) -> Duration { Duration::days(n) }
    fn day(billing_day: Option<u8>) -> crate::settings::Subscription { crate::settings::Subscription { billing_day, ..Default::default() } }

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    fn reading(kind: &str, percent: f64, resets_at: DateTime<Utc>) -> AccountUsage {
        AccountUsage {
            id: "a1".into(), provider: Provider::Claude, email: Some("test@example.invalid".into()), alias: None,
            plan: Some("pro".into()), org: None, active: false, sources: vec![],
            windows: vec![UsageWindow { kind: kind.into(), window_seconds: None, used_percent: percent, resets_at: Some(resets_at) }],
            credits: None, resets: vec![], error: None, monitor_error: None, stale: false, fetched_at: None,
            monitor_connected: true, billing: BillingHints::default(),
        }
    }

    /// A week watched every hour for `hours` hours (spread over the days they span), peaking at `peak`.
    fn week(h: &mut AccountHistory, kind: &str, end: DateTime<Utc>, peak: f64, hours: i64) {
        let start = end - days(7);
        for i in 0..hours {
            let now = start + Duration::hours(i * 7 * 24 / hours.max(1));
            observe(h, &reading(kind, peak * (i + 1) as f64 / hours as f64, end), now);
        }
    }

    fn weeks(peaks: &[f64], hours: i64) -> (AccountHistory, DateTime<Utc>) {
        let mut h = AccountHistory::default();
        let mut end = at("2026-09-07T00:00:00Z");
        for p in peaks { week(&mut h, "weekly", end, *p, hours); end += days(7); }
        (h, end - days(7) + Duration::hours(1))
    }

    #[test]
    fn too_little_history_never_reads_as_unused() {
        assert_eq!(assess(&AccountHistory::default(), Utc::now()).verdict, Verdict::InsufficientData);
        let (one, now) = weeks(&[0.0], 100);
        let a = assess(&one, now);
        assert_eq!((a.verdict, a.weeks_counted, a.renew_wasteful), (Verdict::InsufficientData, 1, false));
        // Four idle weeks glimpsed for three hours each prove nothing.
        let (sparse, now) = weeks(&[0.0; 4], 3);
        assert_eq!(assess(&sparse, now).verdict, Verdict::InsufficientData);
    }

    #[test]
    fn weeks_watched_near_zero_are_unused_and_wasteful_to_renew() {
        let (h, now) = weeks(&[2.0, 0.0, 4.0, 1.0], 100);
        let a = assess(&h, now);
        assert_eq!((a.verdict, a.confidence, a.renew_wasteful), (Verdict::Unused, Confidence::High, true));
    }

    #[test]
    fn weeks_at_the_limit_are_exhausted_on_either_side_of_the_boundary() {
        let (h, now) = weeks(&[100.0, 40.0, 99.5, 60.0], 100);
        let a = assess(&h, now);
        assert_eq!((a.verdict, a.limit_weeks, a.renew_wasteful), (Verdict::ExhaustedWeekly, 2, false));
        let (h, now) = weeks(&[99.4, 40.0, 99.4, 60.0], 100);
        assert_eq!(assess(&h, now).verdict, Verdict::Healthy);
    }

    #[test]
    fn a_model_cap_that_runs_out_counts_but_never_makes_an_account_unused() {
        let (mut h, now) = weeks(&[60.0, 55.0, 70.0, 65.0], 100);
        let mut end = at("2026-09-07T00:00:00Z");
        for _ in 0..4 { week(&mut h, "weekly_opus", end, 100.0, 100); end += days(7); }
        let a = assess(&h, now);
        assert_eq!((a.verdict, a.sub_window_limited.as_deref()), (Verdict::ExhaustedWeekly, Some("weekly_opus")));
        let (mut h, now) = weeks(&[50.0, 45.0, 50.0, 55.0], 100);
        let mut end = at("2026-09-07T00:00:00Z");
        for _ in 0..4 { week(&mut h, "weekly_sonnet", end, 0.0, 100); end += days(7); }
        assert_eq!(assess(&h, now).verdict, Verdict::Healthy);
    }

    #[test]
    fn session_windows_and_odd_lengths_are_not_weekly_records() {
        let mut h = AccountHistory::default();
        let now = at("2026-09-01T00:00:00Z");
        observe(&mut h, &reading("session", 100.0, now + Duration::hours(3)), now);
        let mut odd = reading("weekly", 50.0, now + days(3));
        odd.windows[0].window_seconds = Some(18000);
        observe(&mut h, &odd, now);
        assert!(h.windows.is_empty());
    }

    #[test]
    fn a_low_median_is_underused_and_only_confident_evidence_says_renewal_is_wasteful() {
        let (h, now) = weeks(&[20.0, 10.0, 22.0, 30.0], 100);
        let a = assess(&h, now);
        assert_eq!((a.verdict, a.confidence, a.renew_wasteful), (Verdict::Underused, Confidence::High, true));
        let (h, now) = weeks(&[20.0, 10.0, 22.0], 100);
        let a = assess(&h, now);
        assert_eq!((a.verdict, a.confidence, a.renew_wasteful), (Verdict::Underused, Confidence::Low, false));
    }

    #[test]
    fn stale_failed_and_unseen_readings_leave_history_alone() {
        let now = at("2026-09-01T00:00:00Z");
        let mut h = AccountHistory::default();
        let mut stale = reading("weekly", 80.0, now + days(3));
        stale.stale = true;
        observe(&mut h, &stale, now);
        let mut failed = reading("weekly", 80.0, now + days(3));
        failed.error = Some("http_500".into());
        observe(&mut h, &failed, now);
        assert!(h.first_seen.is_none());
        // A reset that has already passed is not a reading of the next window, let alone 0%.
        observe(&mut h, &reading("weekly", 0.0, now - Duration::hours(1)), now);
        assert!(h.windows.is_empty());
    }

    #[test]
    fn readings_of_one_window_fold_into_one_record_by_hour_and_day() {
        let start = at("2026-09-01T10:00:00Z");
        let end = start + days(5);
        let mut h = AccountHistory::default();
        for (minutes, percent) in [(0, 10.0), (20, 15.0), (50, 12.0), (70, 30.0), (60 * 25, 40.0)] {
            // Jitter in the reported reset must not split the window.
            observe(&mut h, &reading("weekly", percent, end + Duration::seconds(minutes)), start + Duration::minutes(minutes));
        }
        assert_eq!(h.windows.len(), 1);
        let r = &h.windows[0];
        assert_eq!((r.peak, r.hours, r.days), (40.0, 3, 2));
        observe(&mut h, &reading("weekly", 1.0, end + days(7)), end + Duration::hours(1));
        assert_eq!(h.windows.len(), 2);
        assert_eq!(h.windows[0].peak, 40.0);
    }

    /// Polls an idle account hourly for `hours` hours, the way each provider reports it.
    fn idle_polls(h: &mut AccountHistory, start: DateTime<Utc>, hours: i64, codex: bool) {
        for i in 0..hours {
            let now = start + Duration::hours(i);
            let mut acc = reading("weekly", 0.0, now + days(7));
            if codex { acc.windows[0].window_seconds = Some(WEEK_SECONDS); } else { acc.windows[0].resets_at = None; }
            observe(h, &acc, now);
        }
    }

    #[test]
    fn idle_weeks_are_recorded_and_judged_unused() {
        for codex in [true, false] {
            let mut h = AccountHistory::default();
            let start = at("2026-09-01T00:00:00Z");
            idle_polls(&mut h, start, 24 * 28, codex);
            let now = start + days(28) + Duration::hours(1);
            assert_eq!(h.windows.iter().filter(|r| r.resets_at <= now).count(), 4, "four idle weeks, not a record per poll");
            let a = assess(&h, now);
            assert_eq!((a.verdict, a.renew_wasteful), (Verdict::Unused, true));
        }
    }

    #[test]
    fn idle_polls_never_evict_real_weeks_and_usage_ends_the_idle_stretch() {
        let (mut h, now) = weeks(&[90.0; 12], 100);
        idle_polls(&mut h, now, 12, true);
        // Twelve polls make one idle record, which replaces only the single oldest week.
        assert_eq!(h.windows.iter().filter(|r| r.idle).count(), 1);
        assert_eq!(h.windows.iter().filter(|r| !r.idle).count(), KEEP - 1, "idle polls do not churn real weeks out");
        let later = now + Duration::hours(13);
        observe(&mut h, &reading("weekly", 3.0, later + days(7)), later);
        let idle = h.windows.iter().find(|r| r.idle).unwrap();
        assert_eq!(idle.resets_at, later);
        assert!(h.windows.iter().any(|r| !r.idle && r.resets_at > later && r.peak == 3.0));
    }

    #[test]
    fn only_the_latest_weeks_are_kept() {
        let (h, _) = weeks(&[10.0; 13], 2);
        assert_eq!(h.windows.len(), KEEP);
        assert_eq!(h.windows[0].resets_at, at("2026-09-14T00:00:00Z"));
    }

    #[test]
    fn the_store_round_trips_holds_no_identity_and_refuses_what_it_cannot_trust() {
        let dir = TempDir::new();
        let path = dir.0.join("usage-history.json");
        let mut history = History::default();
        let snapshot = Snapshot { accounts: vec![reading("weekly", 42.0, at("2026-09-08T00:00:00Z"))], updated_at: None };
        let valid = ["Claude:a1".to_string()].into_iter().collect();
        record(&mut history, &snapshot, &valid, at("2026-09-02T00:00:00Z"));
        save_to(&path, &history).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains('@'), "{text}");
        assert_eq!(load_from(&path).accounts["Claude:a1"].windows[0].peak, 42.0);
        std::fs::write(&path, br#"{"version":9,"accounts":{"x":{}}}"#).unwrap();
        assert!(load_from(&path).accounts.is_empty());
        record(&mut history, &Snapshot::default(), &Default::default(), at("2026-09-10T00:00:00Z"));
        assert_eq!(history.accounts.len(), 1, "a login missing for a while keeps its history");
        record(&mut history, &Snapshot::default(), &Default::default(), at("2026-09-17T00:00:01Z"));
        assert!(history.accounts.is_empty(), "removed accounts are forgotten after two weeks");
    }

    #[test]
    fn renewal_prefers_the_billing_day_then_the_provider_then_an_estimate() {
        let now = at("2026-10-20T12:00:00Z");
        let mut acc = reading("weekly", 0.0, now);
        assert!(next_renewal(&acc, &day(None), now).is_none());
        acc.billing.started_at = Some(at("2026-08-03T09:00:00Z"));
        let r = next_renewal(&acc, &day(None), now).unwrap();
        assert_eq!((r.at, r.source), (at("2026-11-03T09:00:00Z"), DateSource::Estimated));
        acc.billing.active_until = Some(at("2026-10-28T00:00:00Z"));
        assert_eq!(next_renewal(&acc, &day(None), now).unwrap().source, DateSource::Provider);
        // A provider date that has passed is stale: count on from it, labelled as an estimate.
        acc.billing.active_until = Some(at("2026-10-18T21:26:13Z"));
        let r = next_renewal(&acc, &day(None), now).unwrap();
        assert_eq!((r.at, r.source), (at("2026-11-18T21:26:13Z"), DateSource::Estimated));
        let r = next_renewal(&acc, &day(Some(31)), now).unwrap();
        assert_eq!((r.at, r.source), (at("2026-10-31T00:00:00Z"), DateSource::Entered));
        assert_eq!(next_renewal(&acc, &day(Some(5)), now).unwrap().at, at("2026-11-05T00:00:00Z"));
        assert_eq!(next_renewal(&acc, &day(Some(20)), now).unwrap().at, at("2026-11-20T00:00:00Z"), "today's billing has passed");
        assert_eq!(next_day_of_month(31, at("2027-02-10T00:00:00Z")).unwrap(), at("2027-02-28T00:00:00Z"));
    }

    #[test]
    fn a_corrected_date_wins_and_rolls_on_by_the_plan_period() {
        let now = at("2026-10-20T12:00:00Z");
        let mut acc = reading("weekly", 0.0, now);
        acc.billing.renews_at = Some(at("2026-11-18T21:26:13Z"));
        let fix = |d: &str| crate::settings::Subscription { next_billing: Some(d.parse().unwrap()), ..Default::default() };
        let r = next_renewal(&acc, &fix("2026-11-07"), now).unwrap();
        assert_eq!((r.at, r.source), (at("2026-11-07T00:00:00Z"), DateSource::Entered));
        assert_eq!(next_renewal(&acc, &fix("2026-09-07"), now).unwrap().at, at("2026-11-07T00:00:00Z"), "a past correction counts on monthly");
        assert_eq!(next_renewal(&acc, &fix("2026-10-20"), now).unwrap().at, at("2026-10-20T00:00:00Z"), "billing today");
        acc.billing.billing_period = Some("yearly".into());
        assert_eq!(next_renewal(&acc, &fix("2026-03-01"), now).unwrap().at, at("2027-03-01T00:00:00Z"));
    }

    #[test]
    fn codex_account_record_gives_the_next_charge_and_a_cancellation() {
        let now = at("2026-10-20T12:00:00Z");
        let mut acc = reading("weekly", 0.0, now);
        acc.billing.active_until = Some(at("2026-10-18T21:26:13Z"));
        let record = serde_json::json!({ "accounts": { "acct": {
            "entitlement": { "renews_at": "2026-11-18T21:26:13+00:00", "expires_at": "2026-11-19T03:26:13+00:00", "cancels_at": null,
                "is_delinquent": false, "billing_period": "monthly", "subscription_id": "secret" },
            "last_active_subscription": { "will_renew": true, "cancellation_outcome": null } } } });
        let b = crate::providers::codex::billing(&record, Some("acct")).unwrap();
        acc.billing = acc.billing.clone().overlay(&b);
        let r = next_renewal(&acc, &day(None), now).unwrap();
        assert_eq!((r.at, r.source), (at("2026-11-18T21:26:13Z"), DateSource::Provider));
        assert!(ends_at(&acc, now).is_none());
        assert!(!serde_json::to_string(&acc.billing).unwrap().contains("secret"));
        // The default entry answers when the token's account is not listed by id.
        let fallback = serde_json::json!({ "accounts": { "default": record["accounts"]["acct"].clone() } });
        assert!(crate::providers::codex::billing(&fallback, Some("other")).unwrap().renews_at.is_some());

        let mut cancelled = record.clone();
        cancelled["accounts"]["acct"]["entitlement"]["cancels_at"] = "2026-11-18T21:26:13+00:00".into();
        cancelled["accounts"]["acct"]["last_active_subscription"]["will_renew"] = false.into();
        acc.billing = acc.billing.clone().overlay(&crate::providers::codex::billing(&cancelled, Some("acct")).unwrap());
        assert_eq!(ends_at(&acc, now), Some(at("2026-11-18T21:26:13Z")));
        assert!(next_renewal(&acc, &day(Some(5)), now).is_none(), "a cancelled plan does not renew, whatever day was entered");
    }

    #[test]
    fn plans_prices_and_the_report_totals() {
        let now = at("2026-10-20T12:00:00Z");
        let mut max = reading("weekly", 0.0, now + days(2));
        max.plan = Some("max".into());
        max.billing.tier = Some("default_claude_max_20x".into());
        assert_eq!(plan_label(&max).as_deref(), Some("Max 20x"));
        assert_eq!(list_price(&max), Some(200.0));
        let mut pro = reading("weekly", 0.0, now + days(2));
        pro.provider = Provider::Codex;
        assert_eq!(list_price(&pro), None, "ChatGPT Pro's size is unknown");
        let mut team = reading("weekly", 0.0, now + days(2));
        team.id = "t".into();
        team.plan = Some("team".into());
        assert_eq!(plan_kind(team.plan.as_deref()), PlanKind::Organization);

        let mut settings = Settings::default();
        settings.subscriptions.insert("claude:a1".into(), crate::settings::Subscription { monthly_cost: Some(180.0), ..Default::default() });
        let (h, _) = weeks(&[0.0; 4], 100);
        let mut history = History::default();
        history.accounts.insert("Claude:a1".into(), h);
        // The verdict uses completed weeks only; the report runs after them.
        let report = report(&Snapshot { accounts: vec![max, team], updated_at: None }, &history, &settings, at("2026-10-02T00:00:00Z"));
        let a = &report.accounts[0];
        assert_eq!((a.monthly_cost, a.cost_entered, a.assessment.verdict), (Some(180.0), true, Verdict::Unused));
        assert!(report.accounts[1].renewal.is_none() && report.accounts[1].monthly_cost.is_none());
        let s = &report.providers[0];
        assert_eq!((s.paid, s.unused, s.monthly_total, s.possible_savings), (1, 1, 180.0, 180.0));
    }

    #[test]
    fn pace_compares_usage_with_the_share_of_the_week_gone() {
        let now = at("2026-10-20T12:00:00Z");
        let acc = reading("weekly", 50.0, now + Duration::hours(84));
        assert_eq!(this_week(&acc, now).unwrap().pace_percent, Some(100.0));
        let mut stale = acc.clone();
        stale.stale = true;
        assert_eq!(this_week(&stale, now).unwrap().pace_percent, None);
    }

    #[test]
    fn only_a_passed_last_paid_day_takes_an_account_out_of_rotation() {
        let mut settings = Settings::default();
        settings.subscriptions.insert("claude:a1".into(), crate::settings::Subscription {
            cancel_after: NaiveDate::from_ymd_opt(2026, 10, 28), ..Default::default() });
        assert!(!lapsed(&settings, Provider::Claude, "a1", at("2026-10-28T12:00:00Z")));
        assert!(lapsed(&settings, Provider::Claude, "a1", at("2026-10-29T00:00:00Z")));
        assert!(!lapsed(&settings, Provider::Codex, "a1", at("2026-12-01T00:00:00Z")));
    }
}
