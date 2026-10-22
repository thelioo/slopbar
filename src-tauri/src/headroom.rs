//! Capacity for agents: how long each account's limits last at the pace it is being used, and
//! which account suits a job of a given size. Everything here works from the last snapshot and
//! a short in-memory trail of recent readings; nothing reaches a provider.

use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::accounts::Provider;
use crate::model::{AccountUsage, UsageWindow};
use crate::settings::Settings;

/// Readings older than this say nothing about the current pace.
const TRAIL_MINUTES: i64 = 180;
const TRAIL_MAX: usize = 64;
/// The recent pace needs readings at least this far apart.
const MIN_SPAN_MINUTES: i64 = 20;
/// The window-average pace needs this share of the window gone.
const MIN_ELAPSED_SHARE: f64 = 0.05;
const LIMIT_PERCENT: f64 = 99.5;
/// A reading this many refresh intervals old (and never under `MIN_STALE_MINUTES`) is stale.
const STALE_INTERVALS: i64 = 3;
const MIN_STALE_MINUTES: i64 = 30;
/// An account with less than this left of any window is no candidate for a new job.
const MIN_LEFT_PERCENT: f64 = 5.0;
const SESSION_SECONDS: i64 = 5 * 3600;
const WEEK_SECONDS: i64 = 7 * 86400;

/// Recent readings per account window, newest last; kept only in memory.
#[derive(Debug, Default)]
pub struct Recent {
    trails: HashMap<String, VecDeque<(DateTime<Utc>, f64)>>,
}

fn trail_key(provider: Provider, id: &str, kind: &str) -> String {
    format!("{}:{id}:{kind}", provider.name())
}

impl Recent {
    /// Adds every fresh, successful reading of a poll. A drop in usage means the window reset, so
    /// the trail starts again from it.
    pub fn record(&mut self, accounts: &[AccountUsage], now: DateTime<Utc>) {
        for acc in accounts.iter().filter(|a| !a.stale && a.error.is_none()) {
            let at = acc.fetched_at.unwrap_or(now);
            for w in &acc.windows {
                let trail = self.trails.entry(trail_key(acc.provider, &acc.id, &w.kind)).or_default();
                if trail.back().is_some_and(|(_, p)| w.used_percent < *p) { trail.clear(); }
                if trail.back().is_some_and(|(t, _)| *t >= at) { continue; }
                trail.push_back((at, w.used_percent));
                while trail.len() > TRAIL_MAX || trail.front().is_some_and(|(t, _)| (now - *t).num_minutes() > TRAIL_MINUTES) {
                    trail.pop_front();
                }
            }
        }
        let live: std::collections::HashSet<String> = accounts.iter()
            .flat_map(|a| a.windows.iter().map(|w| trail_key(a.provider, &a.id, &w.kind))).collect();
        self.trails.retain(|k, _| live.contains(k));
    }

    /// Percentage points per hour between the oldest recent reading and the newest.
    fn pace(&self, acc: &AccountUsage, kind: &str, now: DateTime<Utc>) -> Option<f64> {
        let trail = self.trails.get(&trail_key(acc.provider, &acc.id, kind))?;
        let (first, last) = (trail.iter().find(|(t, _)| (now - *t).num_minutes() <= TRAIL_MINUTES)?, trail.back()?);
        let span = (last.0 - first.0).num_seconds();
        (span >= MIN_SPAN_MINUTES * 60).then(|| (last.1 - first.1).max(0.0) * 3600.0 / span as f64)
    }
}

fn window_length(w: &UsageWindow) -> Option<i64> {
    w.window_seconds.or(match w.kind.as_str() {
        "session" => Some(SESSION_SECONDS),
        k if k.starts_with("weekly") => Some(WEEK_SECONDS),
        _ => None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Advice {
    /// Every window lasts until its reset at the current pace.
    Plenty,
    /// No usable reading (stale, failed or never measured).
    Unknown,
    /// At the current pace a window reaches its limit before it resets.
    WillRunOutBeforeReset,
    AtLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaceBasis {
    /// Readings from the past few hours.
    Recent,
    /// Usage so far over the part of the window gone (no recent trail yet).
    WindowAverage,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowHeadroom {
    pub kind: String,
    pub used_percent: f64,
    pub left_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub resets_in_minutes: Option<i64>,
    /// Percentage points used per hour.
    pub pace_per_hour: Option<f64>,
    pub pace_basis: Option<PaceBasis>,
    /// When the limit is reached at that pace; none while nothing is being used.
    pub limit_at: Option<DateTime<Utc>>,
    pub limit_in_minutes: Option<i64>,
    pub advice: Advice,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountHeadroom {
    pub provider: Provider,
    pub id: String,
    /// The alias, else the email.
    pub label: Option<String>,
    pub active: bool,
    pub fetched_at: Option<DateTime<Utc>>,
    pub age_minutes: Option<i64>,
    pub stale: bool,
    pub error: Option<String>,
    pub windows: Vec<WindowHeadroom>,
    /// The worst of the windows' advice.
    pub advice: Advice,
    /// The window that runs out first, when one does before its reset or is at its limit.
    pub binding_window: Option<String>,
}

fn label(acc: &AccountUsage) -> Option<String> {
    acc.alias.clone().or_else(|| acc.email.clone())
}

fn minutes(to: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (to - now).num_minutes().max(0)
}

/// Whether a reading is too old or unreliable to plan with.
pub fn stale(acc: &AccountUsage, settings: &Settings, now: DateTime<Utc>) -> bool {
    let limit = (settings.refresh_minutes.max(1) as i64 * STALE_INTERVALS).max(MIN_STALE_MINUTES);
    acc.stale || acc.fetched_at.map_or(true, |t| (now - t).num_minutes() > limit)
}

pub fn headroom(acc: &AccountUsage, settings: &Settings, recent: &Recent, now: DateTime<Utc>) -> AccountHeadroom {
    let is_stale = stale(acc, settings, now);
    let usable = !is_stale && acc.error.is_none();
    let windows: Vec<WindowHeadroom> = acc.windows.iter().map(|w| {
        let resets_at = w.resets_at.filter(|r| *r > now);
        let average = || {
            let (len, reset) = (window_length(w)?, resets_at?);
            let elapsed = len - (reset - now).num_seconds();
            (elapsed as f64 >= len as f64 * MIN_ELAPSED_SHARE && elapsed <= len).then(|| w.used_percent * 3600.0 / elapsed as f64)
        };
        let (pace, basis) = match (usable, recent.pace(acc, &w.kind, now)) {
            (false, _) => (None, None),
            (true, Some(p)) => (Some(p), Some(PaceBasis::Recent)),
            (true, None) => match average() { Some(p) => (Some(p), Some(PaceBasis::WindowAverage)), None => (None, None) },
        };
        let left = (100.0 - w.used_percent).max(0.0);
        let limit_at = pace.filter(|p| *p > 0.0).map(|p| now + Duration::seconds((left / p * 3600.0) as i64));
        let advice = if !usable {
            Advice::Unknown
        } else if w.used_percent >= LIMIT_PERCENT {
            Advice::AtLimit
        } else if limit_at.zip(resets_at).is_some_and(|(l, r)| l < r) {
            Advice::WillRunOutBeforeReset
        } else {
            Advice::Plenty
        };
        WindowHeadroom {
            kind: w.kind.clone(), used_percent: w.used_percent, left_percent: left,
            resets_at, resets_in_minutes: resets_at.map(|r| minutes(r, now)),
            pace_per_hour: pace.map(|p| (p * 100.0).round() / 100.0), pace_basis: basis,
            limit_at, limit_in_minutes: limit_at.map(|l| minutes(l, now)), advice,
        }
    }).collect();
    let advice = if !usable || windows.is_empty() { Advice::Unknown } else { windows.iter().map(|w| w.advice).max().unwrap_or(Advice::Unknown) };
    let binding_window = windows.iter()
        .filter(|w| matches!(w.advice, Advice::AtLimit | Advice::WillRunOutBeforeReset))
        .min_by_key(|w| if w.advice == Advice::AtLimit { 0 } else { w.limit_in_minutes.unwrap_or(i64::MAX) })
        .map(|w| w.kind.clone());
    AccountHeadroom {
        provider: acc.provider, id: acc.id.clone(), label: label(acc), active: acc.active,
        fetched_at: acc.fetched_at, age_minutes: acc.fetched_at.map(|t| (now - t).num_minutes().max(0)),
        stale: is_stale, error: acc.error.clone(), windows, advice, binding_window,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// The account in use is the best fit.
    Stay,
    /// Another account fits better; `switch: true` (or switch_account) would move to it.
    Switch,
    /// No eligible account has room for the job.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: String,
    pub label: Option<String>,
    pub active: bool,
    /// Has room for the whole job.
    pub fits: bool,
    /// The least left of any weekly window, else of any window.
    pub weekly_left_percent: Option<f64>,
    pub session_left_percent: Option<f64>,
    /// The sooner of the weekly reset and the plan's renewal: unused capacity is lost then.
    pub use_before: Option<DateTime<Utc>>,
    pub use_before_reason: Option<&'static str>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Excluded {
    pub id: String,
    pub label: Option<String>,
    pub active: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub provider: Provider,
    pub expected_percent: Option<f64>,
    pub hours: Option<f64>,
    /// Best first.
    pub ranking: Vec<Candidate>,
    pub excluded: Vec<Excluded>,
    pub action: Action,
    /// The account the recommendation names (the one to use).
    pub recommended: Option<String>,
    pub recommended_label: Option<String>,
    pub summary: String,
}

fn name(label: &Option<String>, id: &str) -> String {
    label.clone().unwrap_or_else(|| id.to_string())
}

/// Why an account cannot take a job, or None when it can be ranked.
fn exclusion(acc: &AccountUsage, settings: &Settings, now: DateTime<Utc>) -> Option<String> {
    if acc.error.is_some() { return Some("its last reading failed".into()); }
    if stale(acc, settings, now) { return Some("its reading is stale".into()); }
    if acc.windows.is_empty() { return Some("no usage measured yet".into()); }
    if let Some(seat) = acc.remote.iter().find(|r| r.live) {
        return Some(format!("in use on another machine ({})", seat.machine));
    }
    if crate::subscriptions::lapsed(settings, acc.provider, &acc.id, now) { return Some("its plan has lapsed".into()); }
    let prefs = settings.subscriptions.get(&crate::subscriptions::pref_key(acc.provider, &acc.id));
    if prefs.and_then(|p| p.cancel_after).is_some() || crate::subscriptions::ends_at(acc, now).is_some() {
        return Some("its plan is set to end".into());
    }
    if acc.windows.iter().any(|w| w.used_percent >= LIMIT_PERCENT) { return Some("a limit is reached".into()); }
    None
}

/// Ranks the provider's accounts for a job needing `expected_percent` of a week over `hours`.
pub fn recommend(accounts: &[AccountUsage], settings: &Settings, provider: Provider, expected_percent: Option<f64>, hours: Option<f64>, now: DateTime<Utc>) -> Recommendation {
    let job_end = hours.map(|h| now + Duration::seconds((h * 3600.0) as i64));
    let mut ranking = Vec::new();
    let mut excluded = Vec::new();
    for acc in accounts.iter().filter(|a| a.provider == provider) {
        let label = label(acc);
        if let Some(reason) = exclusion(acc, settings, now) {
            excluded.push(Excluded { id: acc.id.clone(), label, active: acc.active, reason });
            continue;
        }
        let weekly: Vec<&UsageWindow> = acc.windows.iter().filter(|w| w.kind.starts_with("weekly")).collect();
        let left = |ws: &[&UsageWindow]| ws.iter().map(|w| (100.0 - w.used_percent).max(0.0)).reduce(f64::min);
        let weekly_left = left(&weekly).or_else(|| left(&acc.windows.iter().collect::<Vec<_>>()));
        let session_left = left(&acc.windows.iter().filter(|w| w.kind == "session").collect::<Vec<_>>());
        let mut reasons = Vec::new();
        // A weekly reset inside the job refills the window: only the part before it must fit now.
        let need_now = expected_percent.map(|need| {
            let reset = weekly.iter().filter_map(|w| w.resets_at).filter(|r| *r > now).min();
            match (reset, job_end, hours) {
                (Some(r), Some(end), Some(h)) if r < end && h > 0.0 => {
                    reasons.push("its weekly limit resets during the job".into());
                    need * (r - now).num_seconds() as f64 / (h * 3600.0)
                }
                _ => need,
            }
        });
        let fits = weekly_left.is_some_and(|l| l >= need_now.unwrap_or(MIN_LEFT_PERCENT).max(MIN_LEFT_PERCENT))
            && session_left.map_or(true, |l| l >= MIN_LEFT_PERCENT);
        match (weekly_left, need_now) {
            (Some(l), Some(n)) if l >= n => reasons.push(format!("{l:.0}% of the week left, the job needs about {n:.0}%")),
            (Some(l), Some(n)) => reasons.push(format!("only {l:.0}% of the week left, the job needs about {n:.0}%")),
            (Some(l), None) => reasons.push(format!("{l:.0}% of the week left")),
            _ => {}
        }
        if session_left.is_some_and(|l| l < MIN_LEFT_PERCENT) { reasons.push("its session limit is nearly used".into()); }
        if acc.used_outside { reasons.push("its usage rose outside this PC recently".into()); }
        let weekly_reset = weekly.iter().filter_map(|w| w.resets_at).filter(|r| *r > now).min();
        let prefs = settings.subscriptions.get(&crate::subscriptions::pref_key(acc.provider, &acc.id)).cloned().unwrap_or_default();
        let renewal = crate::subscriptions::next_renewal(acc, &prefs, now).map(|r| r.at);
        let (use_before, why) = match (weekly_reset, renewal) {
            (Some(w), Some(r)) if r < w => (Some(r), Some("renewal")),
            (Some(w), _) => (Some(w), Some("weekly_reset")),
            (None, Some(r)) => (Some(r), Some("renewal")),
            (None, None) => (None, None),
        };
        if let Some(at) = use_before {
            let days = (at - now).num_hours() as f64 / 24.0;
            reasons.push(format!("unused capacity is lost at its {} in {days:.1} days", if why == Some("renewal") { "renewal" } else { "weekly reset" }));
        }
        ranking.push(Candidate { id: acc.id.clone(), label, active: acc.active, fits, weekly_left_percent: weekly_left, session_left_percent: session_left, use_before, use_before_reason: why, reasons });
    }
    // Fitting accounts first; among them the one that loses its capacity soonest, then the most left.
    ranking.sort_by(|a, b| b.fits.cmp(&a.fits)
        .then_with(|| if a.fits { a.use_before.unwrap_or(DateTime::<Utc>::MAX_UTC).cmp(&b.use_before.unwrap_or(DateTime::<Utc>::MAX_UTC)) } else { std::cmp::Ordering::Equal })
        .then_with(|| b.weekly_left_percent.unwrap_or(0.0).total_cmp(&a.weekly_left_percent.unwrap_or(0.0)))
        .then_with(|| b.active.cmp(&a.active)));
    let best = ranking.first().filter(|c| c.fits);
    let (action, summary) = match best {
        Some(c) if c.active => (Action::Stay, format!("Keep using {}: it fits the job and should be used first.", name(&c.label, &c.id))),
        Some(c) => (Action::Switch, format!("Switch to {}: it fits the job and its capacity is lost soonest. Switch only if the user allowed switching.", name(&c.label, &c.id))),
        None => (Action::None, "No eligible account has room for this job; wait for a reset or ask the user.".into()),
    };
    Recommendation {
        provider, expected_percent, hours,
        recommended: best.map(|c| c.id.clone()), recommended_label: best.and_then(|c| c.label.clone()),
        action, summary, ranking, excluded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RemoteSeat;

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }
    const NOW: &str = "2026-10-21T12:00:00Z";

    fn window(kind: &str, used: f64, resets_in_hours: i64) -> UsageWindow {
        UsageWindow { kind: kind.into(), window_seconds: None, used_percent: used, resets_at: Some(at(NOW) + Duration::hours(resets_in_hours)) }
    }

    fn account(id: &str, active: bool, weekly: f64, weekly_resets_in_hours: i64) -> AccountUsage {
        AccountUsage {
            id: id.into(), provider: Provider::Claude, email: Some(format!("{id}@example.invalid")), alias: None, plan: Some("max".into()), org: None,
            active, sources: vec![], windows: vec![window("session", 10.0, 3), window("weekly", weekly, weekly_resets_in_hours)],
            credits: None, resets: vec![], error: None, monitor_error: None, stale: false,
            fetched_at: Some(at(NOW) - Duration::minutes(2)), monitor_connected: true, billing: Default::default(), remote: vec![], used_outside: false, runs_out_at: None,
        }
    }

    #[test]
    fn forecasts_from_recent_readings_and_falls_back_to_the_window_average() {
        let now = at(NOW);
        let settings = Settings::default();
        // 40% used with half the week gone: 0.48 points an hour, limit in 126 hours, after the reset in 84.
        let mut acc = account("a", true, 40.0, 84);
        acc.windows.truncate(0);
        acc.windows.push(window("weekly", 40.0, 84));
        let plain = headroom(&acc, &settings, &Recent::default(), now);
        assert_eq!(plain.windows[0].pace_basis, Some(PaceBasis::WindowAverage));
        assert_eq!(plain.advice, Advice::Plenty);
        // Recently it rose 10 points in an hour: at that pace the week runs out in about six hours.
        let mut recent = Recent::default();
        let mut earlier = acc.clone();
        earlier.windows[0].used_percent = 30.0;
        earlier.fetched_at = Some(now - Duration::minutes(62));
        recent.record(&[earlier], now - Duration::minutes(62));
        recent.record(&[acc.clone()], now);
        let busy = headroom(&acc, &settings, &recent, now);
        assert_eq!(busy.windows[0].pace_basis, Some(PaceBasis::Recent));
        assert_eq!(busy.advice, Advice::WillRunOutBeforeReset);
        assert_eq!(busy.binding_window.as_deref(), Some("weekly"));
        assert!((330..=400).contains(&busy.windows[0].limit_in_minutes.unwrap()), "{:?}", busy.windows[0].limit_in_minutes);
        acc.windows[0].used_percent = 100.0;
        assert_eq!(headroom(&acc, &settings, &recent, now).advice, Advice::AtLimit);
        acc.stale = true;
        assert_eq!(headroom(&acc, &settings, &recent, now).advice, Advice::Unknown);
        acc.stale = false;
        acc.fetched_at = Some(now - Duration::hours(2));
        assert!(headroom(&acc, &settings, &recent, now).stale, "an old reading counts as stale");
    }

    #[test]
    fn a_reset_restarts_the_trail() {
        let now = at(NOW);
        let mut recent = Recent::default();
        let mut acc = account("a", true, 90.0, 1);
        acc.fetched_at = Some(now - Duration::minutes(60));
        recent.record(&[acc.clone()], now - Duration::minutes(60));
        acc.windows[1].used_percent = 2.0;
        acc.fetched_at = Some(now);
        recent.record(&[acc.clone()], now);
        assert_eq!(recent.pace(&acc, "weekly", now), None);
    }

    #[test]
    fn ranks_fitting_accounts_by_the_capacity_lost_soonest() {
        let now = at(NOW);
        let settings = Settings::default();
        let current = account("current", true, 40.0, 120);
        let soon = account("soon", false, 50.0, 20);
        let roomy = account("roomy", false, 5.0, 150);
        let tight = account("tight", false, 97.0, 2);
        let rec = recommend(&[current.clone(), soon.clone(), roomy.clone(), tight.clone()], &settings, Provider::Claude, Some(30.0), Some(8.0), now);
        let order: Vec<&str> = rec.ranking.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(order, ["soon", "current", "roomy", "tight"]);
        assert!(!rec.ranking[3].fits);
        assert_eq!(rec.action, Action::Switch);
        assert_eq!(rec.recommended.as_deref(), Some("soon"));
        assert!(rec.summary.contains("soon@example.invalid"));
        // A bigger job no longer fits "soon": its reset falls after the job, so all of it must fit now.
        let big = recommend(&[current.clone(), soon.clone(), roomy.clone()], &settings, Provider::Claude, Some(70.0), Some(8.0), now);
        assert_eq!(big.recommended.as_deref(), Some("roomy"));
        // But a reset inside a long job means only the share before it must fit.
        let long = recommend(&[account("nearly", false, 92.0, 2)], &settings, Provider::Claude, Some(50.0), Some(24.0), now);
        assert!(long.ranking[0].fits, "{:?}", long.ranking[0].reasons);
        // An alias labels the account in place of its email.
        let mut aliased = soon.clone();
        aliased.alias = Some("Spare".into());
        let named = recommend(&[current.clone(), aliased], &settings, Provider::Claude, None, None, now);
        assert!(named.summary.contains("Spare"));
        // When the account in use is the best fit, stay.
        let stay = recommend(&[current.clone(), roomy], &settings, Provider::Claude, Some(10.0), None, now);
        assert_eq!(stay.action, Action::Stay);
        assert_eq!(recommend(&[tight], &settings, Provider::Claude, Some(30.0), Some(1.0), now).action, Action::None);
    }

    #[test]
    fn excludes_remote_lapsed_ending_failed_and_stale_accounts() {
        let now = at(NOW);
        let mut settings = Settings::default();
        let mut remote = account("remote", false, 0.0, 10);
        remote.remote = vec![RemoteSeat { machine: "DESKTOP-TEST".into(), where_: vec!["Windows".into()], seen_at: now, live: true }];
        let mut quiet = account("quiet-remote", false, 0.0, 10);
        quiet.remote = vec![RemoteSeat { machine: "DESKTOP-TEST".into(), where_: vec![], seen_at: now - Duration::hours(5), live: false }];
        let lapsed = account("lapsed", false, 0.0, 10);
        settings.subscriptions.insert("claude:lapsed".into(), crate::settings::Subscription { cancel_after: Some((now - Duration::days(1)).date_naive()), ..Default::default() });
        let mut ending = account("ending", false, 0.0, 10);
        ending.billing.will_renew = Some(false);
        ending.billing.active_until = Some(now + Duration::days(3));
        let mut failed = account("failed", false, 0.0, 10);
        failed.error = Some("token_expired".into());
        let mut old = account("old", false, 0.0, 10);
        old.fetched_at = Some(now - Duration::hours(3));
        let mut other = account("codex", false, 0.0, 10);
        other.provider = Provider::Codex;
        let rec = recommend(&[remote, quiet, lapsed, ending, failed, old, other], &settings, Provider::Claude, None, None, now);
        assert_eq!(rec.ranking.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["quiet-remote"]);
        let why: HashMap<&str, &str> = rec.excluded.iter().map(|e| (e.id.as_str(), e.reason.as_str())).collect();
        assert_eq!(why.len(), 5);
        assert!(why["remote"].contains("DESKTOP-TEST"));
        assert!(why["lapsed"].contains("lapsed"));
        assert!(why["ending"].contains("end"));
        assert!(why["failed"].contains("failed"));
        assert!(why["old"].contains("stale"));
    }

    #[test]
    fn renewal_before_the_weekly_reset_sets_use_before() {
        let now = at(NOW);
        let mut settings = Settings::default();
        let acc = account("a", false, 10.0, 100);
        settings.subscriptions.insert("claude:a".into(), crate::settings::Subscription { next_billing: Some((now + Duration::days(2)).date_naive()), ..Default::default() });
        let rec = recommend(&[acc], &settings, Provider::Claude, None, None, now);
        assert_eq!(rec.ranking[0].use_before_reason, Some("renewal"));
    }
}
