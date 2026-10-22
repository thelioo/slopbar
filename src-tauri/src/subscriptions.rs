//! Whether each paid plan earns its keep: a small history of every completed weekly window per
//! account (how far it got before resetting, and how much of it we actually saw), a verdict from
//! that history, and the next renewal with where that date came from.
//!
//! Nothing here guesses usage. A week we did not watch is missing, never 0%: an account is only
//! called unused after several weeks of real readings near zero. SlopBar cannot cancel a plan;
//! the report only says which ones look unneeded and links to the provider's own billing page.

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    /// Codex: the purchased credit balance last read, and what has been spent since SlopBar first
    /// read one (see `note_credits`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<CreditMark>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreditMark {
    pub balance: f64,
    pub at: DateTime<Utc>,
    /// Every drop in the balance since `since`. A top-up raises the balance without resetting it.
    pub spent: f64,
    pub since: DateTime<Utc>,
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
    /// Read from the Codex CLI's rollouts on this PC rather than polled by SlopBar. Never stored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
}

impl WindowRecord {
    pub(crate) fn hit_limit(&self) -> bool { self.peak >= LIMIT_PERCENT }
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
    if let Some(balance) = acc.credits.as_deref().and_then(parse_balance) { note_credits(h, balance, now); }
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
                first_obs: now, last_obs: now, hours: 1, days: 1, idle: false, local: false }, now),
        }
    }
}

fn observe_idle(h: &mut AccountHistory, kind: &str, now: DateTime<Utc>) {
    match h.windows.iter_mut().find(|r| r.kind == kind && r.resets_at > now) {
        // An open window already holds this week: a 0% reading leaves its peak alone.
        Some(r) => seen(r, now),
        None => push(h, WindowRecord { kind: kind.into(), resets_at: now + chrono::Duration::seconds(WEEK_SECONDS),
            peak: 0.0, first_obs: now, last_obs: now, hours: 1, days: 1, idle: true, local: false }, now),
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
pub(crate) fn pref_key(provider: Provider, id: &str) -> String {
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

/// What the verdict rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    /// Weeks SlopBar polled itself.
    Observed,
    /// Partly or wholly the Codex CLI's history on this PC, which misses use anywhere else.
    LocalCodex,
}

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
    pub basis: Basis,
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
    let basis = if weeks.iter().any(|r| r.local) { Basis::LocalCodex } else { Basis::Observed };
    Assessment { verdict, confidence, weeks_counted: counted, weeks_needed: 2, average_peak, median_peak,
        limit_weeks, sub_window_limited, renew_wasteful, basis }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        Some("pro" | "prolite" | "promax" | "max" | "plus" | "go") => PlanKind::Personal,
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
        // The Codex CLI's own names for the three ChatGPT Pro tiers.
        (Provider::Codex, "prolite") => "Pro 100".into(),
        (Provider::Codex, "pro") => "Pro 200".into(),
        (Provider::Codex, "promax") => "Pro 500".into(),
        (_, p) => { let mut c = p.chars(); c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default() }
    })
}

/// US list price per month, when the plan name settles it. Codex reports ChatGPT Pro's tier as its
/// plan type (prolite, pro, promax), so each Pro tier has its price.
pub fn list_price(acc: &AccountUsage) -> Option<f64> {
    Some(match (acc.provider, plan_label(acc)?.as_str()) {
        (Provider::Claude, "Pro") => 20.0,
        (Provider::Claude, "Max 5x") => 100.0,
        (Provider::Claude, "Max 20x") => 200.0,
        (Provider::Codex, "Plus") => 20.0,
        (Provider::Codex, "Pro 100") => 100.0,
        (Provider::Codex, "Pro 200") => 200.0,
        (Provider::Codex, "Pro 500") => 500.0,
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
    /// When the week reaches its limit at the current pace, when that is before it resets
    /// (`headroom::runs_out_at`).
    pub runs_out_at: Option<DateTime<Utc>>,
}

fn this_week(acc: &AccountUsage, now: DateTime<Utc>) -> Option<ThisWeek> {
    let w = acc.windows.iter().find(|w| w.kind == "weekly")?;
    let elapsed = w.resets_at.map(|r| WEEK_SECONDS - (r - now).num_seconds()).filter(|e| *e > 6 * 3600 && *e <= WEEK_SECONDS);
    Some(ThisWeek {
        used_percent: w.used_percent,
        resets_at: w.resets_at,
        pace_percent: if acc.stale { None } else { elapsed.map(|e| w.used_percent * WEEK_SECONDS as f64 / e as f64) },
        runs_out_at: if acc.stale { None } else { acc.runs_out_at },
    })
}

/// What a paid plan's next renewal deserves a warning for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalWarning {
    /// Renews within `WASTEFUL_DAYS` while it looks unused or underused.
    Wasteful,
    /// Renews within a day.
    Soon,
}

pub const WASTEFUL_DAYS: i64 = 3;

/// OpenAI: grandfathered ChatGPT Pro 200 subscribers keep the previous Codex allowance through
/// this day; after it Pro 200 includes a lower allowance at the same price.
/// https://help.openai.com/en/articles/9793128-about-chatgpt-pro-tiers
pub const PRO_200_LAST_DAY: (i32, u32, u32) = (2026, 10, 29);
/// The new allowance as a share of the old. OpenAI gives no figure; third-party estimates put it
/// at about half.
pub const PRO_200_NEW_ALLOWANCE: f64 = 0.5;

/// Pro 200's coming allowance cut, for one account on it.
#[derive(Debug, Clone, Serialize)]
pub struct AllowanceChange {
    /// The last day of the previous allowance.
    pub last_day: NaiveDate,
    /// The highest weekly usage in recent weeks, the week in progress included.
    pub peak_percent: f64,
    /// That peak measured against the new allowance.
    pub projected_percent: f64,
    /// That week would reach the limit under the new allowance.
    pub would_hit_limit: bool,
}

/// For a Codex account on Pro 200 before the cut: whether its recent weeks would fit the smaller
/// allowance. None for other plans, after the cut, or with no week to go on.
pub fn allowance_change(acc: &AccountUsage, h: &AccountHistory, now: DateTime<Utc>) -> Option<AllowanceChange> {
    let (y, m, d) = PRO_200_LAST_DAY;
    let last_day = NaiveDate::from_ymd_opt(y, m, d)?;
    if acc.provider != Provider::Codex || acc.plan.as_deref() != Some("pro") || now.date_naive() > last_day { return None; }
    let mut done: Vec<&WindowRecord> = h.windows.iter().filter(|r| r.kind == "weekly" && r.resets_at <= now && (r.counts() || r.local)).collect();
    done.sort_by_key(|r| std::cmp::Reverse(r.resets_at));
    let live = acc.windows.iter().filter(|w| w.kind == "weekly" && !acc.stale && acc.error.is_none()).map(|w| w.used_percent);
    let peak = done.iter().take(RECENT).map(|r| r.peak).chain(live).reduce(f64::max)?;
    let projected = peak / PRO_200_NEW_ALLOWANCE;
    Some(AllowanceChange { last_day, peak_percent: peak, projected_percent: (projected * 10.0).round() / 10.0, would_hit_limit: projected >= 100.0 })
}

fn renewal_warning(renewal: Option<&Renewal>, ends_at: Option<DateTime<Utc>>, a: &Assessment, now: DateTime<Utc>) -> Option<RenewalWarning> {
    let left = renewal?.at - now;
    if ends_at.is_some() || left <= chrono::Duration::zero() { return None; }
    let wasteful = a.renew_wasteful || matches!(a.verdict, Verdict::Unused | Verdict::Underused);
    if wasteful && left <= chrono::Duration::days(WASTEFUL_DAYS) {
        Some(RenewalWarning::Wasteful)
    } else if left <= chrono::Duration::hours(24) {
        Some(RenewalWarning::Soon)
    } else {
        None
    }
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
    /// The renewal is close enough to warn about (see `RenewalWarning`).
    pub renewal_warning: Option<RenewalWarning>,
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
    /// The provider's own plan name ("chatgptpro"), when it reported one.
    pub plan_code: Option<String>,
    /// The live weekly reading shows no use in the past seven days: 0% with its reset a full week out.
    pub idle_now: bool,
    /// Codex only: what the Codex CLI on this PC shows for this account.
    pub local: Option<crate::codex_local::LocalActivity>,
    /// Codex Pro 200 only, until its allowance is cut (see `allowance_change`).
    pub allowance_change: Option<AllowanceChange>,
    /// This plan's weekly allowance in plans of the provider's reference tier (`plan_weight`).
    pub plan_weight: Option<f64>,
    /// What the plan costs per use, when its cost is known (`cost_per_use`).
    pub cost_per_use: Option<CostPerUse>,
    /// Codex only: purchased credits and their trend.
    pub credits: Option<CreditInfo>,
    /// Claude only: extra usage beyond the plan this month, when the provider reported it.
    pub extra_usage: Option<crate::model::ExtraUsage>,
    /// Inputs for the provider's pool sizing; not part of the report.
    #[serde(skip)]
    pool_weeks: BTreeMap<NaiveDate, f64>,
    /// Weeks of `pool_weeks` read from this PC's Codex rollouts.
    #[serde(skip)]
    pool_local: BTreeSet<NaiveDate>,
    /// The week SlopBar started watching the account.
    #[serde(skip)]
    pool_since: Option<NaiveDate>,
}

impl Entry {
    /// The busiest recent week in percent of this plan's allowance: the latest `RECENT` weeks that
    /// count, and the week in progress.
    pub(crate) fn recent_peak(&self) -> Option<f64> {
        let live = self.this_week.as_ref().filter(|_| !self.stale).map(|w| w.used_percent);
        self.pool_weeks.values().rev().take(RECENT).copied().chain(live).reduce(f64::max)
    }
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
    /// How many plans the provider's usage needs (`pool`).
    pub pool: Pool,
    /// Blended cost per use across its paid plans (`blended_cost`).
    pub cost_per_use: BlendedCost,
    /// Codex: purchased credits across its accounts, when any reported a balance.
    pub credit_balance: Option<f64>,
    /// Claude: extra usage spent this month across the accounts that have it turned on, in
    /// dollars, on top of `monthly_total`; None when none has.
    pub extra_usage_spent: Option<f64>,
    pub extra_usage_limit: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub accounts: Vec<Entry>,
    pub providers: Vec<ProviderSummary>,
    pub generated_at: DateTime<Utc>,
    /// Plan changes per provider (`advisor`).
    #[serde(default)]
    pub advice: Vec<crate::advisor::ProviderAdvice>,
    /// What the plans have cost month by month (`ledger`), when the app has a ledger.
    pub spend: Option<crate::ledger::Summary>,
}

/// A Codex plan with no use in the live week and none on this PC for this long reads as unused
/// while SlopBar's own history is still too short, labelled as resting on local history.
const LOCAL_IDLE_DAYS: i64 = 14;

/// Whether the live weekly window shows nothing used in the past week.
fn idle_now(acc: &AccountUsage, now: DateTime<Utc>) -> bool {
    if acc.stale || acc.error.is_some() { return false; }
    let read_at = acc.fetched_at.unwrap_or(now);
    acc.windows.iter().any(|w| w.kind == "weekly" && w.used_percent <= 0.0 && w.window_seconds.map_or(true, |s| s == WEEK_SECONDS)
        && w.resets_at.is_some_and(|r| ((r - read_at).num_seconds() - WEEK_SECONDS).abs() <= IDLE_SLACK_SECONDS))
}

/// Adds the weeks read from local rollouts that SlopBar did not watch itself. A week it watched
/// wins: its own readings span the whole week, the rollouts only the hours Codex ran here.
fn with_local(h: &AccountHistory, extra: &[WindowRecord]) -> AccountHistory {
    let span = |r: &WindowRecord| (if r.idle { r.first_obs } else { r.resets_at - chrono::Duration::seconds(WEEK_SECONDS) }, r.resets_at);
    let mut merged = h.clone();
    for w in extra {
        let (s, e) = span(w);
        let watched = h.windows.iter().filter(|r| r.kind == w.kind).any(|r| {
            let (rs, re) = span(r);
            (e.min(re) - s.max(rs)).num_hours() >= 24
        });
        if !watched { merged.windows.push(w.clone()); }
    }
    merged
}

pub fn report(snapshot: &Snapshot, history: &History, settings: &Settings, local: Option<&crate::codex_local::Local>, now: DateTime<Utc>) -> Report {
    let empty = AccountHistory::default();
    let accounts: Vec<Entry> = snapshot.accounts.iter().map(|acc| {
        let prefs = settings.subscriptions.get(&pref_key(acc.provider, &acc.id)).cloned().unwrap_or_default();
        let h = history.accounts.get(&history_key(acc.provider, &acc.id)).unwrap_or(&empty);
        let kind = plan_kind(acc.plan.as_deref());
        let paid = kind == PlanKind::Personal;
        let monthly_cost = prefs.monthly_cost.or_else(|| list_price(acc).filter(|_| paid));
        let local = local.filter(|_| acc.provider == Provider::Codex);
        let merged = local.map(|l| with_local(h, l.weeks_for(&acc.id)));
        let h = merged.as_ref().unwrap_or(h);
        let activity = local.and_then(|l| l.activity(&acc.id, now));
        let idle_now = idle_now(acc, now);
        let mut assessment = assess(h, now);
        // This PC's history cannot see other machines: an account another SlopBar reports in use,
        // or whose usage rose while not in use here, is not called unused on local evidence.
        let seen_elsewhere = !acc.remote.is_empty() || acc.used_outside;
        if paid && assessment.verdict == Verdict::InsufficientData && idle_now && !seen_elsewhere
            && activity.as_ref().is_some_and(|a| a.days_idle.is_some_and(|d| d >= LOCAL_IDLE_DAYS)) {
            assessment.verdict = Verdict::Unused;
            assessment.confidence = Confidence::Low;
            assessment.renew_wasteful = true;
            assessment.basis = Basis::LocalCodex;
        }
        let renewal = if paid { next_renewal(acc, &prefs, now) } else { None };
        let ends_at = if paid { ends_at(acc, now) } else { None };
        let cost_per_use = monthly_cost.map(|c| cost_per_use(c, &assessment, activity.as_ref()));
        let (pool_weeks, pool_local) = week_peaks(h, now);
        // Only from when SlopBar watched it: before then, rollout weeks exist only for the accounts
        // used on this PC, and leaving the rest out would make the fleet look less busy.
        let pool_since = h.first_seen.map(|t| monday(t.date_naive()));
        let credits = (acc.provider == Provider::Codex).then(|| credit_info(acc.credits.as_deref(), h.credits.as_ref())).flatten();
        Entry {
            renewal_warning: if paid { renewal_warning(renewal.as_ref(), ends_at, &assessment, now) } else { None },
            provider: acc.provider,
            id: acc.id.clone(),
            email: acc.email.clone(),
            alias: acc.alias.clone(),
            plan: plan_label(acc),
            plan_kind: kind,
            active: acc.active,
            stale: acc.stale,
            this_week: this_week(acc, now),
            assessment,
            days_tracked: h.first_seen.map_or(0, |t| (now - t).num_days().max(0)),
            renewal,
            provider_until: acc.billing.active_until,
            provider_checked: acc.billing.checked_at,
            store_billed: acc.billing.billing_type.as_deref().is_some_and(|t| t.contains("apple") || t.contains("google") || t.contains("app_store") || t.contains("play")),
            cost_entered: prefs.monthly_cost.is_some(),
            monthly_cost,
            cancel_after: prefs.cancel_after,
            ends_at,
            delinquent: acc.billing.delinquent == Some(true),
            plan_code: acc.billing.plan_code.clone(),
            idle_now,
            local: activity,
            allowance_change: allowance_change(acc, h, now),
            plan_weight: plan_weight(acc),
            cost_per_use,
            credits,
            extra_usage: acc.extra_usage.clone().filter(|_| acc.provider == Provider::Claude),
            pool_weeks,
            pool_local,
            pool_since,
        }
    }).collect();
    let providers: Vec<ProviderSummary> = Provider::ALL.into_iter().filter(|p| accounts.iter().any(|a| a.provider == *p)).map(|provider| {
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
            pool: pool(provider, &paid, now),
            cost_per_use: blended_cost(&paid),
            credit_balance: accounts.iter().filter(|a| a.provider == provider).filter_map(|a| a.credits.as_ref()?.balance)
                .fold(None, |t, b| Some(t.unwrap_or(0.0) + b)),
            extra_usage_spent: sum_extra(&accounts, provider, |x| x.used),
            extra_usage_limit: sum_extra(&accounts, provider, |x| x.monthly_limit),
        }
    }).collect();
    let advice = crate::advisor::advise(&accounts, &providers);
    Report { accounts, providers, generated_at: now, advice, spend: None }
}

fn sum_extra(accounts: &[Entry], provider: Provider, f: impl Fn(&crate::model::ExtraUsage) -> Option<f64>) -> Option<f64> {
    accounts.iter().filter(|a| a.provider == provider).filter_map(|a| a.extra_usage.as_ref().filter(|x| x.active()).and_then(&f))
        .fold(None, |t, v| Some(t.unwrap_or(0.0) + v))
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

// ---------------------------------------------------------------------------------------------
// Pool sizing, cost per use and purchased credits

/// The plan pool sizing counts in for each provider: its largest common tier.
pub fn reference_plan(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Max 20x",
        Provider::Codex => "Pro 200",
    }
}

/// One week of this plan's allowance in weeks of the reference plan ("plan-weeks"), so usage on
/// different tiers adds up. Assumptions, since neither provider publishes absolute allowances:
/// where the provider states the multiple, that multiple (Claude sells Max as 5x and 20x Pro's
/// usage, so Pro = 1/20 and Max 5x = 1/4 of Max 20x); otherwise the list price ratio (ChatGPT
/// Plus = 0.1, Pro 100 = 0.5 and Pro 500 = 2.5 of Pro 200).
pub fn plan_weight(acc: &AccountUsage) -> Option<f64> {
    Some(match (acc.provider, plan_label(acc)?.as_str()) {
        (Provider::Claude, "Pro") => 0.05,
        (Provider::Claude, "Max 5x") => 0.25,
        (Provider::Claude, "Max 20x") => 1.0,
        (Provider::Codex, "Plus") => 0.1,
        (Provider::Codex, "Pro 100") => 0.5,
        (Provider::Codex, "Pro 200") => 1.0,
        (Provider::Codex, "Pro 500") => 2.5,
        _ => return None,
    })
}

/// Pool sizing looks at this many of the latest weeks, needs `POOL_MIN_WEEKS` of them, takes the
/// `POOL_PERCENTILE` busiest and adds `POOL_BUFFER` on top.
pub const POOL_WEEKS: usize = 8;
pub const POOL_MIN_WEEKS: u8 = 2;
pub const POOL_PERCENTILE: f64 = 0.9;
pub const POOL_BUFFER: f64 = 0.2;
/// A week counts only when accounts holding at least this share of the plans were watched in it.
const POOL_COVERAGE: f64 = 0.5;

fn monday(d: NaiveDate) -> NaiveDate {
    d - chrono::Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// Each completed weekly window watched closely enough to count, by the Monday of the UTC week
/// it reset in: accounts reset on different days, but each has one window ending per week. When
/// an idle stretch and a real window end in the same week, the higher peak.
fn week_peaks(h: &AccountHistory, now: DateTime<Utc>) -> (BTreeMap<NaiveDate, f64>, BTreeSet<NaiveDate>) {
    let mut out = BTreeMap::new();
    let mut local = BTreeSet::new();
    for r in h.windows.iter().filter(|r| r.kind == "weekly" && r.resets_at <= now && r.counts()) {
        let week = monday((r.resets_at - chrono::Duration::seconds(1)).date_naive());
        let peak = out.entry(week).or_insert(0.0);
        *peak = f64::max(*peak, r.peak);
        if r.local { local.insert(week); }
    }
    (out, local)
}

/// How many plans a provider's usage needs: "your usage fits in N plans; you hold M".
#[derive(Debug, Clone, Serialize)]
pub struct Pool {
    /// The plan the numbers count in (`reference_plan`).
    pub unit: &'static str,
    /// Paid plans held, in that unit.
    pub plans_held: f64,
    /// Paid plans whose tier has no known weight, left out of every number.
    pub unweighted: usize,
    /// Recent completed weeks the demand rests on, out of `POOL_WEEKS`.
    pub weeks_used: u8,
    pub weeks_needed: u8,
    /// Of those, weeks where an account not watched was filled in from its own typical week.
    pub weeks_estimated: u8,
    /// The busiest weeks' total use (90th percentile), in plan-weeks, or this week's so far when
    /// that is already higher. None with fewer than `POOL_MIN_WEEKS` weeks: not enough history.
    pub demand_p90: Option<f64>,
    /// This week's use so far across the plans, in plan-weeks.
    pub live_demand: Option<f64>,
    /// Plans that cover `demand_p90` with `POOL_BUFFER` to spare.
    pub plans_needed: Option<u32>,
    /// How far `plans_needed` exceeds `demand_p90`, in percent.
    pub spare_percent: Option<f64>,
    pub confidence: Confidence,
    /// `LocalCodex` when a counted week includes one read from this PC's Codex rollouts.
    pub basis: Basis,
}

/// Sums each recent week's peak across the paid plans, weighted by `plan_weight`, and sizes the
/// pool from its 90th percentile. A week counts when accounts holding at least half the plans
/// (of those SlopBar knew by then) were watched in it; an account missing from a counted week is
/// filled in with the median of its own counted weeks, and the confidence says so.
fn pool(provider: Provider, paid: &[&Entry], now: DateTime<Utc>) -> Pool {
    let weighted: Vec<(&Entry, f64)> = paid.iter().filter_map(|e| Some((*e, e.plan_weight?))).collect();
    let plans_held = weighted.iter().map(|(_, w)| w).sum::<f64>();
    let medians: Vec<Option<f64>> = weighted.iter().map(|(e, _)| median(&e.pool_weeks.values().copied().collect::<Vec<_>>())).collect();
    let all: BTreeSet<NaiveDate> = weighted.iter().flat_map(|(e, _)| e.pool_weeks.keys().copied()).collect();
    let mut demands = Vec::new();
    let mut estimated = 0u8;
    let mut local = false;
    for week in all.iter().rev().take(POOL_WEEKS) {
        let known: Vec<usize> = (0..weighted.len()).filter(|&i| weighted[i].0.pool_since.is_some_and(|s| s <= *week)).collect();
        let held: f64 = known.iter().map(|&i| weighted[i].1).sum();
        let watched: f64 = known.iter().filter(|&&i| weighted[i].0.pool_weeks.contains_key(week)).map(|&i| weighted[i].1).sum();
        if held <= 0.0 || watched < held * POOL_COVERAGE { continue; }
        let mut filled = false;
        let demand: f64 = known.iter().map(|&i| {
            let (e, w) = weighted[i];
            match e.pool_weeks.get(week) {
                Some(p) => p / 100.0 * w,
                None => { filled = true; medians[i].unwrap_or(0.0) / 100.0 * w }
            }
        }).sum();
        estimated += filled as u8;
        local |= known.iter().any(|&i| weighted[i].0.pool_local.contains(week));
        demands.push(demand);
    }
    let weeks_used = demands.len() as u8;
    let live: Vec<f64> = weighted.iter().filter(|(e, _)| !e.stale)
        .filter_map(|(e, w)| e.this_week.as_ref().filter(|t| t.resets_at.map_or(true, |r| r > now)).map(|t| t.used_percent / 100.0 * w)).collect();
    let live_demand = (!live.is_empty()).then(|| live.iter().sum::<f64>());
    let demand_p90 = (weeks_used >= POOL_MIN_WEEKS).then(|| {
        let p = percentile(&demands, POOL_PERCENTILE);
        live_demand.map_or(p, |l| p.max(l))
    });
    let plans_needed = demand_p90.map(|d| ((d * (1.0 + POOL_BUFFER) - 1e-9).ceil() as u32).max(1));
    let spare_percent = demand_p90.zip(plans_needed).filter(|(d, _)| *d > 0.0).map(|(d, n)| (n as f64 / d - 1.0) * 100.0);
    let confidence = if weeks_used >= RECENT as u8 && estimated == 0 { Confidence::High } else { Confidence::Low };
    Pool { unit: reference_plan(provider), plans_held, unweighted: paid.len() - weighted.len(), weeks_used,
        weeks_needed: POOL_MIN_WEEKS, weeks_estimated: estimated, demand_p90, live_demand, plans_needed, spare_percent, confidence,
        basis: if local { Basis::LocalCodex } else { Basis::Observed } }
}

/// Nearest-rank percentile: with fewer than ten weeks the 90th is the busiest week.
fn percentile(values: &[f64], p: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = ((p * v.len() as f64).ceil() as usize).clamp(1, v.len().max(1));
    v.get(rank - 1).copied().unwrap_or(0.0)
}

/// A month in weeks and days, for spreading a monthly price over use.
const WEEKS_PER_MONTH: f64 = 4.35;
const DAYS_PER_MONTH: f64 = 30.44;

/// What a plan costs for the use it gets.
#[derive(Debug, Clone, Serialize)]
pub struct CostPerUse {
    /// Dollars per 1% of this plan's weekly allowance used: the monthly cost over a month
    /// (`WEEKS_PER_MONTH`) of its average weekly peak. None until the verdict has its weeks, or
    /// when nothing was used.
    pub per_percent: Option<f64>,
    /// Codex: dollars per thread the Codex CLI on this PC ran for it in the last 28 days, the
    /// monthly cost scaled to 28 days. This PC only: threads elsewhere are not counted.
    pub per_local_thread: Option<f64>,
}

fn cost_per_use(monthly: f64, a: &Assessment, local: Option<&crate::codex_local::LocalActivity>) -> CostPerUse {
    let used = a.average_peak.filter(|p| *p > 0.0 && a.verdict != Verdict::InsufficientData);
    CostPerUse {
        per_percent: used.map(|p| monthly / (WEEKS_PER_MONTH * p)),
        per_local_thread: local.filter(|l| l.threads_28d > 0).map(|l| monthly * 28.0 / DAYS_PER_MONTH / l.threads_28d as f64),
    }
}

/// Cost per use across a provider's paid plans.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BlendedCost {
    /// Dollars per 1% of one reference plan's week used (`Pool::unit`): the cost of every plan
    /// with a verdict over their weighted use, so idle plans raise it.
    pub per_plan_percent: Option<f64>,
    /// Codex: dollars per thread on this PC over the plans it has threads for. This PC only.
    pub per_local_thread: Option<f64>,
}

fn blended_cost(paid: &[&Entry]) -> BlendedCost {
    let judged: Vec<(f64, f64)> = paid.iter().filter(|e| e.assessment.verdict != Verdict::InsufficientData)
        .filter_map(|e| Some((e.monthly_cost?, e.assessment.average_peak? * e.plan_weight?))).collect();
    let used: f64 = judged.iter().map(|(_, u)| u).sum();
    let local: Vec<(f64, u32)> = paid.iter().filter_map(|e| Some((e.monthly_cost?, e.local.as_ref()?.threads_28d))).collect();
    let threads: u32 = local.iter().map(|(_, t)| t).sum();
    BlendedCost {
        per_plan_percent: (used > 0.0).then(|| judged.iter().map(|(c, _)| c).sum::<f64>() / (WEEKS_PER_MONTH * used)),
        per_local_thread: (threads > 0).then(|| local.iter().map(|(c, _)| c).sum::<f64>() * 28.0 / DAYS_PER_MONTH / threads as f64),
    }
}

/// Purchased Codex credits as the report shows them. No dollar value: OpenAI's Codex pricing page
/// (https://learn.chatgpt.com/docs/pricing, checked 2026-10-21) says credit purchase prices
/// "depend on your plan or agreement", and the $40 per 1,000 credits seen in third-party posts is
/// not confirmed by OpenAI, so a balance is never priced here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CreditInfo {
    /// The live balance, when the provider reported a number.
    pub balance: Option<f64>,
    pub unlimited: bool,
    /// Spent since `since`, from the balances SlopBar has read.
    pub spent: Option<f64>,
    pub since: Option<DateTime<Utc>>,
}

fn parse_balance(raw: &str) -> Option<f64> {
    raw.trim().replace(',', "").parse::<f64>().ok().filter(|b| b.is_finite() && *b >= 0.0)
}

/// Keeps the latest balance and adds every drop to what was spent. A rise is a purchase: it is
/// not spending, and the count carries on from the new balance. Credits that expire count as spent.
fn note_credits(h: &mut AccountHistory, balance: f64, now: DateTime<Utc>) {
    match &mut h.credits {
        Some(m) => {
            if balance < m.balance { m.spent += m.balance - balance; }
            m.balance = balance;
            m.at = now;
        }
        None => h.credits = Some(CreditMark { balance, at: now, spent: 0.0, since: now }),
    }
}

fn credit_info(live: Option<&str>, mark: Option<&CreditMark>) -> Option<CreditInfo> {
    let unlimited = live == Some("unlimited");
    let balance = live.and_then(parse_balance);
    if !unlimited && balance.is_none() && mark.is_none() { return None; }
    Some(CreditInfo {
        // Only the live figure: Codex reports none once the credits run out.
        balance,
        unlimited,
        spent: mark.filter(|_| !unlimited).map(|m| m.spent),
        since: mark.filter(|_| !unlimited).map(|m| m.since),
    })
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
            monitor_connected: true, billing: BillingHints::default(), remote: vec![], used_outside: false, runs_out_at: None, unexpected: None, cli_rejected: false, extra_usage: None,
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
        assert_eq!((plan_label(&pro).as_deref(), list_price(&pro)), (Some("Pro 200"), Some(200.0)));
        for (plan, label, price) in [("prolite", "Pro 100", 100.0), ("promax", "Pro 500", 500.0), ("plus", "Plus", 20.0)] {
            pro.plan = Some(plan.into());
            assert_eq!((plan_label(&pro).as_deref(), list_price(&pro), plan_kind(pro.plan.as_deref())), (Some(label), Some(price), PlanKind::Personal));
        }
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
        let report = report(&Snapshot { accounts: vec![max, team], updated_at: None }, &history, &settings, None, at("2026-10-02T00:00:00Z"));
        let a = &report.accounts[0];
        assert_eq!((a.monthly_cost, a.cost_entered, a.assessment.verdict), (Some(180.0), true, Verdict::Unused));
        assert!(report.accounts[1].renewal.is_none() && report.accounts[1].monthly_cost.is_none());
        let s = &report.providers[0];
        assert_eq!((s.paid, s.unused, s.monthly_total, s.possible_savings), (1, 1, 180.0, 180.0));
    }

    fn codex_pro(id: &str, used: f64, resets_in: Duration, now: DateTime<Utc>) -> AccountUsage {
        let mut acc = reading("weekly", used, now + resets_in);
        acc.id = id.into();
        acc.provider = Provider::Codex;
        acc.windows[0].window_seconds = Some(WEEK_SECONDS);
        acc.fetched_at = Some(now);
        acc
    }

    fn local_codex(now: DateTime<Utc>) -> crate::codex_local::Local {
        let mut local = crate::codex_local::Local { attributed_since: Some(now - days(26)), ..Default::default() };
        let busy = crate::codex_local::Activity { threads: 5, tokens: 9000, last: Some(now - days(1)),
            days: [((now - days(1)).date_naive(), (5, 9000))].into_iter().collect() };
        local.accounts.insert("acct-busy".into(), busy);
        local
    }

    #[test]
    fn a_long_idle_codex_plan_reads_unused_from_local_history_and_says_so() {
        let now = at("2026-10-21T12:00:00Z");
        let local = local_codex(now);
        let snapshot = Snapshot { accounts: vec![
            codex_pro("u1:acct-idle", 0.0, days(7), now),
            codex_pro("u2:acct-busy", 0.0, days(7), now),
            codex_pro("u3:acct-elsewhere", 12.0, days(3), now),
        ], updated_at: None };
        let r = report(&snapshot, &History::default(), &Settings::default(), Some(&local), now);
        let idle = &r.accounts[0];
        assert!(idle.idle_now);
        let l = idle.local.as_ref().unwrap();
        assert_eq!((l.never_seen, l.days_idle, l.threads_28d), (true, Some(26), 0));
        let a = &idle.assessment;
        assert_eq!((a.verdict, a.basis, a.confidence, a.renew_wasteful), (Verdict::Unused, Basis::LocalCodex, Confidence::Low, true));
        // The same quiet account, but its usage rose while not in use here: local history is blind.
        let mut elsewhere = snapshot.clone();
        elsewhere.accounts[0].used_outside = true;
        let seen = report(&elsewhere, &History::default(), &Settings::default(), Some(&local), now);
        assert_eq!((seen.accounts[0].assessment.verdict, seen.accounts[0].assessment.renew_wasteful), (Verdict::InsufficientData, false));
        // Ran here yesterday: not idle locally, however quiet the live week looks.
        assert_eq!(r.accounts[1].assessment.verdict, Verdict::InsufficientData);
        assert_eq!(r.accounts[1].local.as_ref().unwrap().threads_7d, 5);
        // Used this week somewhere this PC cannot see: never called unused.
        assert!(!r.accounts[2].idle_now);
        assert_eq!(r.accounts[2].assessment.verdict, Verdict::InsufficientData);
        // Without local history there is no such verdict at all.
        let r = report(&snapshot, &History::default(), &Settings::default(), None, now);
        assert!(r.accounts[0].local.is_none());
        assert_eq!(r.accounts[0].assessment.verdict, Verdict::InsufficientData);
    }

    #[test]
    fn rollout_weeks_fill_gaps_but_never_replace_watched_weeks() {
        let now = at("2026-10-21T12:00:00Z");
        let rec = |end: &str, peak: f64, local: bool| WindowRecord { kind: "weekly".into(), resets_at: at(end), peak,
            first_obs: at(end) - days(6), last_obs: at(end) - days(1), hours: 40, days: 5, idle: false, local };
        let mut local = local_codex(now);
        local.weeks.insert("acct-busy".into(), vec![rec("2026-10-03T00:00:00Z", 100.0, true), rec("2026-10-10T00:00:00Z", 100.0, true), rec("2026-10-17T00:00:00Z", 100.0, true)]);
        let mut history = History::default();
        history.accounts.insert("Codex:u2:acct-busy".into(), AccountHistory { windows: vec![rec("2026-10-17T00:20:00Z", 30.0, false)], ..Default::default() });
        let snapshot = Snapshot { accounts: vec![codex_pro("u2:acct-busy", 50.0, days(3), now)], updated_at: None };
        let a = &report(&snapshot, &history, &Settings::default(), Some(&local), now).accounts[0].assessment;
        assert_eq!((a.weeks_counted, a.limit_weeks, a.verdict, a.basis), (3, 2, Verdict::ExhaustedWeekly, Basis::LocalCodex));
        let text = serde_json::to_string(&history).unwrap();
        assert!(!text.contains("local"), "local weeks are never written to the history file");
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

    /// A fresh weekly reading taken `now`, `elapsed_hours` into its week.
    fn live(used: f64, elapsed_hours: i64, now: DateTime<Utc>) -> AccountUsage {
        let mut acc = reading("weekly", used, now + Duration::hours(7 * 24 - elapsed_hours));
        acc.fetched_at = Some(now);
        acc
    }

    /// The one forecast (`headroom::runs_out_at`), with no recent readings unless given.
    fn forecast(acc: &AccountUsage, recent: Option<&crate::headroom::Recent>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        crate::headroom::runs_out_at(acc, &Settings::default(), recent.unwrap_or(&Default::default()), now)
    }

    #[test]
    fn the_forecast_runs_out_before_the_reset_only_on_a_fast_pace() {
        let now = at("2026-10-20T12:00:00Z");
        // 60% two days in: 1.25%/h, so the last 40% lasts 32 h, well before the reset in 5 days.
        let f = forecast(&live(60.0, 48, now), None, now).unwrap();
        assert_eq!(f, now + Duration::hours(32));
        // 30% four days in is on pace to finish under 100%: no forecast.
        assert!(forecast(&live(30.0, 96, now), None, now).is_none());
        // The first hours of a week, an empty week and a spent one say nothing.
        assert!(forecast(&live(20.0, 3, now), None, now).is_none());
        assert!(forecast(&live(0.0, 48, now), None, now).is_none());
        assert!(forecast(&live(100.0, 48, now), None, now).is_none());
        // Session windows are not the week.
        let mut session = live(60.0, 48, now);
        session.windows[0].kind = "session".into();
        assert!(forecast(&session, None, now).is_none());
    }

    #[test]
    fn recent_readings_override_the_week_average_in_a_burst() {
        let start = at("2026-10-20T08:00:00Z");
        let resets = start + Duration::hours(4 * 24 + 2);
        let mut recent = crate::headroom::Recent::default();
        // Three days in at 30%, then 10 points in the last two hours: 5%/h now against 0.56%/h overall.
        for (minutes, used) in [(0, 30.0), (60, 35.0), (120, 40.0)] {
            let mut acc = reading("weekly", used, resets);
            acc.fetched_at = Some(start + Duration::minutes(minutes));
            recent.record(&[acc], start + Duration::minutes(minutes));
        }
        let now = start + Duration::hours(2);
        let mut acc = reading("weekly", 40.0, resets);
        acc.fetched_at = Some(now);
        assert!(forecast(&acc, None, now).is_none(), "the weekly average alone finishes in time");
        // The recent pace wins outright: 60 points at 5%/h is 12 hours.
        let f = forecast(&acc, Some(&recent), now).unwrap();
        assert_eq!(f, now + Duration::hours(12));
    }

    #[test]
    fn stale_failed_and_old_readings_never_forecast() {
        let now = at("2026-10-20T12:00:00Z");
        let mut stale = live(60.0, 48, now);
        stale.stale = true;
        stale.runs_out_at = Some(now + Duration::hours(3));
        assert!(forecast(&stale, None, now).is_none());
        assert!(this_week(&stale, now).unwrap().runs_out_at.is_none(), "a stale reading shows no stored forecast either");
        let mut failed = live(60.0, 48, now);
        failed.error = Some("http_500".into());
        assert!(forecast(&failed, None, now).is_none());
        let mut old = live(60.0, 48, now - Duration::hours(1));
        assert!(forecast(&old, None, now).is_none(), "read an hour ago");
        old.fetched_at = None;
        assert!(forecast(&old, None, now).is_none(), "never read");
        // The forecast reaches get_usage and the report.
        let mut snapshot = Snapshot { accounts: vec![live(60.0, 48, now)], updated_at: Some(now) };
        crate::headroom::attach_forecasts(&mut snapshot, &Settings::default(), &Default::default(), now);
        assert_eq!(snapshot.accounts[0].runs_out_at, Some(now + Duration::hours(32)));
        let r = report(&snapshot, &History::default(), &Settings::default(), None, now);
        assert_eq!(r.accounts[0].this_week.as_ref().unwrap().runs_out_at, Some(now + Duration::hours(32)));
    }

    #[test]
    fn renewal_warnings_name_unneeded_plans_three_days_out_and_any_plan_a_day_out() {
        let now = at("2026-10-20T12:00:00Z");
        let (unused, _) = weeks(&[0.0; 4], 100);
        let unused = assess(&unused, at("2026-10-02T00:00:00Z"));
        let (busy, _) = weeks(&[70.0; 4], 100);
        let busy = assess(&busy, at("2026-10-02T00:00:00Z"));
        let r = |hours: i64| Renewal { at: now + Duration::hours(hours), source: DateSource::Provider };
        assert_eq!(renewal_warning(Some(&r(70)), None, &unused, now), Some(RenewalWarning::Wasteful));
        assert_eq!(renewal_warning(Some(&r(73)), None, &unused, now), None);
        assert_eq!(renewal_warning(Some(&r(70)), None, &busy, now), None);
        assert_eq!(renewal_warning(Some(&r(20)), None, &busy, now), Some(RenewalWarning::Soon));
        assert_eq!(renewal_warning(Some(&r(20)), Some(now + Duration::hours(20)), &unused, now), None, "cancelled");
        assert_eq!(renewal_warning(None, None, &unused, now), None);
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

    fn max(id: &str, tier: &str, now: DateTime<Utc>, used: f64) -> AccountUsage {
        let mut acc = reading("weekly", used, now + days(2));
        acc.id = id.into();
        acc.plan = Some("max".into());
        acc.billing.tier = Some(format!("default_claude_max_{tier}"));
        acc
    }

    /// A Claude fleet: each account's weekly peaks, aligned on the same weeks.
    fn fleet(plans: &[(&str, &str, &[f64])]) -> (Snapshot, History, DateTime<Utc>) {
        let mut history = History::default();
        let mut now = at("2000-01-01T00:00:00Z");
        let mut accounts = vec![];
        for (id, tier, peaks) in plans {
            let (h, end) = weeks(peaks, 100);
            now = now.max(end);
            history.accounts.insert(format!("Claude:{id}"), h);
            accounts.push(max(id, tier, end, 0.0));
        }
        (Snapshot { accounts, updated_at: None }, history, now)
    }

    #[test]
    fn the_pool_fits_the_busiest_week_of_the_fleet_with_a_buffer() {
        let (snapshot, history, now) = fleet(&[
            ("a", "20x", &[50.0, 60.0, 40.0, 70.0]),
            ("b", "20x", &[10.0, 20.0, 10.0, 30.0]),
            ("c", "20x", &[0.0, 0.0, 0.0, 0.0]),
        ]);
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        // Weekly totals 0.6, 0.8, 0.5 and 1.0 plans: the busiest of four is the 90th percentile.
        assert_eq!((p.unit, p.plans_held, p.weeks_used, p.weeks_estimated), ("Max 20x", 3.0, 4, 0));
        assert!((p.demand_p90.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!((p.plans_needed, p.confidence), (Some(2), Confidence::High));
        assert!((p.spare_percent.unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(p.live_demand, Some(0.0));
    }

    #[test]
    fn the_pool_says_not_enough_history_below_two_weeks() {
        let (snapshot, history, now) = fleet(&[("a", "20x", &[90.0]), ("b", "20x", &[90.0])]);
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.weeks_used, p.demand_p90, p.plans_needed, p.spare_percent), (1, None, None, None));
        assert_eq!((p.plans_held, p.confidence), (2.0, Confidence::Low));
        let p = &report(&snapshot, &History::default(), &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.weeks_used, p.plans_needed), (0, None));
    }

    #[test]
    fn tiers_count_in_plans_of_the_largest_and_unknown_tiers_stay_out() {
        assert_eq!(plan_weight(&max("a", "5x", Utc::now(), 0.0)), Some(0.25));
        assert_eq!(plan_weight(&reading("weekly", 0.0, Utc::now())), Some(0.05));
        let now = Utc::now();
        for (plan, w) in [("prolite", 0.5), ("pro", 1.0), ("promax", 2.5), ("plus", 0.1)] {
            let mut c = codex_pro("u:a", 0.0, days(2), now);
            c.plan = Some(plan.into());
            assert_eq!(plan_weight(&c), Some(w));
        }
        // A Max 5x running out every week is a quarter of a Max 20x: one plan covers it.
        let (mut snapshot, history, now) = fleet(&[("small", "5x", &[100.0, 100.0, 100.0]), ("big", "20x", &[0.0, 0.0, 0.0])]);
        let mut odd = max("odd", "20x", now, 0.0);
        odd.billing.tier = Some("default_claude_max_7x".into());
        snapshot.accounts.push(odd);
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.plans_held, p.unweighted, p.plans_needed), (1.25, 1, Some(1)));
        assert!((p.demand_p90.unwrap() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_missed_week_is_filled_from_the_accounts_own_weeks_and_lowers_confidence() {
        let (snapshot, mut history, now) = fleet(&[("a", "20x", &[80.0; 4]), ("b", "20x", &[20.0, 40.0, 30.0, 60.0]), ("c", "20x", &[10.0; 4])]);
        let second = at("2026-09-14T00:00:00Z");
        // b was not watched in the second week: filled in with its median (30%).
        history.accounts.get_mut("Claude:b").unwrap().windows.retain(|r| r.resets_at != second);
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.weeks_used, p.weeks_estimated, p.confidence), (4, 1, Confidence::Low));
        assert!((p.demand_p90.unwrap() - 1.5).abs() < 1e-9);
        // With c missing too, fewer than half the plans were watched: the week does not count.
        history.accounts.get_mut("Claude:c").unwrap().windows.retain(|r| r.resets_at != second);
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.weeks_used, p.weeks_estimated), (3, 0));
    }

    #[test]
    fn weeks_before_slopbar_watched_the_fleet_do_not_count_and_local_weeks_say_so() {
        let (snapshot, mut history, now) = fleet(&[("a", "20x", &[80.0, 80.0, 80.0]), ("b", "20x", &[10.0, 10.0, 10.0])]);
        // b was only first seen in the last week: the two before it cannot size the pool.
        history.accounts.get_mut("Claude:b").unwrap().first_seen = Some(at("2026-09-15T00:00:00Z"));
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.weeks_used, p.basis), (3, Basis::Observed));
        for h in history.accounts.values_mut() { h.first_seen = Some(at("2026-09-15T00:00:00Z")); }
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.weeks_used, p.plans_needed), (1, None));
        // A week read from this PC's rollouts counts, and the pool says it rests on them.
        for h in history.accounts.values_mut() { h.first_seen = Some(at("2026-08-31T00:00:00Z")); }
        history.accounts.get_mut("Claude:a").unwrap().windows[0].local = true;
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert_eq!((p.weeks_used, p.basis), (3, Basis::LocalCodex));
    }

    #[test]
    fn a_busier_week_in_progress_raises_the_demand() {
        let (mut snapshot, history, now) = fleet(&[("a", "20x", &[30.0, 30.0]), ("b", "20x", &[30.0, 30.0])]);
        snapshot.accounts[0].windows[0].used_percent = 95.0;
        snapshot.accounts[1].windows[0].used_percent = 90.0;
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert!((p.live_demand.unwrap() - 1.85).abs() < 1e-9 && (p.demand_p90.unwrap() - 1.85).abs() < 1e-9);
        assert_eq!(p.plans_needed, Some(3));
        // A stale reading is not this week's use.
        snapshot.accounts[0].stale = true;
        let p = &report(&snapshot, &history, &Settings::default(), None, now).providers[0].pool;
        assert!((p.demand_p90.unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn percentiles_rank_to_the_nearest_week() {
        assert_eq!(percentile(&[], 0.9), 0.0);
        assert_eq!(percentile(&[3.0, 1.0, 2.0], 0.9), 3.0);
        let ten: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(percentile(&ten, 0.9), 9.0);
    }

    #[test]
    fn cost_per_use_spreads_the_price_over_what_was_used() {
        let (snapshot, history, now) = fleet(&[("a", "20x", &[50.0, 50.0]), ("b", "5x", &[0.0, 0.0]), ("new", "20x", &[10.0])]);
        let r = report(&snapshot, &history, &Settings::default(), None, now);
        let per = |i: usize| r.accounts[i].cost_per_use.as_ref().unwrap().per_percent;
        assert!((per(0).unwrap() - 200.0 / (4.35 * 50.0)).abs() < 1e-9);
        // Nothing used, or too little history to have an average: no figure.
        assert_eq!((per(1), per(2)), (None, None));
        // Blended over the plans with a verdict, in percent of one Max 20x week: the idle Max 5x adds cost, not use.
        let blended = r.providers[0].cost_per_use.per_plan_percent.unwrap();
        assert!((blended - 300.0 / (4.35 * 50.0)).abs() < 1e-9);
        assert!(r.providers[0].cost_per_use.per_local_thread.is_none());
        // Free and unpriced plans have no cost per use.
        let mut free = max("f", "20x", now, 0.0);
        free.plan = Some("free".into());
        let r = report(&Snapshot { accounts: vec![free], updated_at: None }, &History::default(), &Settings::default(), None, now);
        assert!(r.accounts[0].cost_per_use.is_none() && r.providers[0].cost_per_use.per_plan_percent.is_none());
    }

    #[test]
    fn codex_cost_per_thread_counts_this_pcs_threads_only() {
        let now = at("2026-10-21T12:00:00Z");
        let local = local_codex(now);
        let snapshot = Snapshot { accounts: vec![codex_pro("u1:acct-idle", 0.0, days(7), now), codex_pro("u2:acct-busy", 5.0, days(3), now)], updated_at: None };
        let r = report(&snapshot, &History::default(), &Settings::default(), Some(&local), now);
        assert_eq!(r.accounts[0].cost_per_use.as_ref().unwrap().per_local_thread, None);
        let busy = r.accounts[1].cost_per_use.as_ref().unwrap().per_local_thread.unwrap();
        assert!((busy - 200.0 * 28.0 / 30.44 / 5.0).abs() < 1e-9);
        let blended = r.providers[0].cost_per_use.per_local_thread.unwrap();
        assert!((blended - 400.0 * 28.0 / 30.44 / 5.0).abs() < 1e-9);
        // Without this PC's history there is nothing to count.
        let r = report(&snapshot, &History::default(), &Settings::default(), None, now);
        assert!(r.providers[0].cost_per_use.per_local_thread.is_none());
    }

    #[test]
    fn credits_track_what_was_spent_across_a_top_up() {
        let start = at("2026-10-01T00:00:00Z");
        let mut acc = codex_pro("u:a", 10.0, days(3), start);
        let mut h = AccountHistory::default();
        for (i, balance) in ["100", "80", "1,120", "1,110"].into_iter().enumerate() {
            acc.credits = Some(balance.into());
            observe(&mut h, &acc, start + Duration::hours(i as i64));
        }
        // A stale reading or none at all changes nothing.
        acc.stale = true;
        acc.credits = Some("5".into());
        observe(&mut h, &acc, start + days(1));
        let m = h.credits.clone().unwrap();
        assert_eq!((m.balance, m.spent, m.since), (1110.0, 30.0, start));
        let info = credit_info(Some("1110"), Some(&m)).unwrap();
        assert_eq!((info.balance, info.unlimited, info.spent, info.since), (Some(1110.0), false, Some(30.0), Some(start)));
        // Unlimited has no balance and nothing to spend; no credits ever and none now is no entry.
        let info = credit_info(Some("unlimited"), None).unwrap();
        assert_eq!((info.balance, info.unlimited, info.spent), (None, true, None));
        assert_eq!(credit_info(None, None), None);
        assert_eq!(credit_info(Some("n/a"), None), None);
        // The report shows them for Codex accounts and totals the balances.
        let now = start + days(2);
        let mut acc = codex_pro("u:a", 10.0, days(3), now);
        acc.credits = Some("1110".into());
        let mut history = History::default();
        history.accounts.insert("Codex:u:a".into(), h);
        let r = report(&Snapshot { accounts: vec![acc], updated_at: None }, &history, &Settings::default(), None, now);
        assert_eq!(r.accounts[0].credits.as_ref().and_then(|c| c.spent), Some(30.0));
        assert_eq!(r.providers[0].credit_balance, Some(1110.0));
        // The trend survives a save.
        let json = serde_json::to_string(&history).unwrap();
        let back: History = serde_json::from_str(&json).unwrap();
        assert_eq!(back.accounts["Codex:u:a"].credits.as_ref().map(|c| c.spent), Some(30.0));
    }
}
