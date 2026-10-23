//! Runaway agents: an account burning through its week abnormally fast. Either its weekly window
//! rose by more than the user's threshold within the set minutes (15 points in 60 by default), or
//! one repository and model on this PC is using several times its usual tokens per window. The
//! alert names the account, the repository and model behind it here (from `usage_index`, so only
//! what this PC's CLIs wrote) and when the week runs out at that pace. Each account is notified
//! at most every few hours; what was seen in the past day stays readable for agents.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::headroom::Recent;
use crate::model::AccountUsage;
use crate::settings::Settings;
use crate::usage_index::LocalRate;

/// At most one notification per account in this many hours.
pub const NOTIFY_EVERY_H: i64 = 3;
/// What was seen is kept this long for `get_burn_alerts`.
const KEEP_H: i64 = 24;
/// The local baseline: how far back a repository and model's usual rate is read.
pub const BASELINE_DAYS: i64 = 7;
/// A repository and model needs this many tokens in the window, and this many windows of use
/// before it, for its rate to count as abnormal: a burst from a quiet repo is not a runaway.
const MIN_LOCAL_TOKENS: u64 = 2_000_000;
const MIN_BASELINE_WINDOWS: u32 = 3;
const LIMIT_PERCENT: f64 = 99.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The weekly window rose by the threshold within the set minutes.
    WeeklyRise,
    /// A repository and model on this PC used several times its usual tokens.
    LocalRate,
}

/// What this PC's CLIs show behind the burn: the busiest repository and model in the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Culprit {
    pub repo: String,
    pub model: String,
    pub tokens: u64,
    /// Its usual tokens per window of the same length, while in use.
    pub baseline: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BurnAlert {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub trigger: Trigger,
    pub detected_at: DateTime<Utc>,
    /// The weekly window's rise and over how many minutes (weekly_rise).
    pub rise_points: Option<f64>,
    pub rise_minutes: Option<i64>,
    pub used_percent: Option<f64>,
    /// Weekly percentage points per hour at this pace.
    pub pace_per_hour: Option<f64>,
    /// When the weekly limit is reached at this pace, when that is before it resets.
    pub limit_at: Option<DateTime<Utc>>,
    pub resets_at: Option<DateTime<Utc>>,
    /// The repository and model behind it, from this PC's files only.
    pub this_pc: Option<Culprit>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub last: Option<BurnAlert>,
    pub notified_at: Option<DateTime<Utc>>,
}

/// Keyed "provider:account id".
pub type Log = HashMap<String, Entry>;

fn key(provider: Provider, id: &str) -> String {
    format!("{}:{}", provider.key(), id)
}

/// The accounts burning abnormally fast now. `rates` comes from `usage_index::local_rates` over a
/// window of `settings.burn.minutes`.
pub fn detect(accounts: &[AccountUsage], recent: &Recent, rates: &[LocalRate], settings: &Settings, now: DateTime<Utc>) -> Vec<BurnAlert> {
    let b = &settings.burn;
    let minutes = b.minutes as i64;
    let mut out = vec![];
    for acc in accounts.iter().filter(|a| !crate::headroom::stale(a, settings, now) && a.error.is_none()) {
        let Some(weekly) = acc.windows.iter().find(|w| w.kind == "weekly") else { continue };
        let mine: Vec<&LocalRate> = rates.iter().filter(|r| r.provider == acc.provider && r.id.as_deref() == Some(acc.id.as_str()) && r.recent > 0).collect();
        let culprit = |r: &LocalRate| Culprit { repo: r.repo.clone(), model: r.model.clone(), tokens: r.recent, baseline: r.baseline };
        let rise = recent.rise(acc.provider, &acc.id, "weekly", minutes, now).filter(|(p, _)| *p >= b.points as f64);
        let unusual = mine.iter().find(|r| r.recent >= MIN_LOCAL_TOKENS && r.active_windows >= MIN_BASELINE_WINDOWS
            && r.baseline.is_some_and(|base| r.recent as f64 >= base * b.token_factor as f64));
        let (trigger, this_pc, pace) = match (rise, unusual) {
            (Some((points, span)), _) => (Trigger::WeeklyRise, mine.first().map(|r| culprit(r)), Some(points * 60.0 / span as f64)),
            (None, Some(r)) => {
                let pace = crate::headroom::headroom(acc, settings, recent, now).windows.into_iter()
                    .find(|w| w.kind == "weekly").and_then(|w| w.pace_per_hour);
                (Trigger::LocalRate, Some(culprit(r)), pace)
            }
            (None, None) => continue,
        };
        let read_at = acc.fetched_at.unwrap_or(now);
        let limit_at = if weekly.used_percent >= LIMIT_PERCENT { Some(read_at) } else {
            pace.filter(|p| *p > 0.0).map(|p| read_at + Duration::seconds(((LIMIT_PERCENT - weekly.used_percent) / p * 3600.0) as i64))
                .filter(|t| weekly.resets_at.is_none_or(|r| *t < r))
        };
        out.push(BurnAlert {
            provider: acc.provider, id: acc.id.clone(), email: acc.email.clone(), alias: acc.alias.clone(), trigger, detected_at: now,
            rise_points: rise.map(|r| round(r.0)), rise_minutes: rise.map(|r| r.1), used_percent: Some(weekly.used_percent),
            pace_per_hour: pace.map(round), limit_at, resets_at: weekly.resets_at, this_pc,
        });
    }
    out
}

/// Logs what `detect` found and returns the alerts due a notification now: with `notify` on, at
/// most one per account every `NOTIFY_EVERY_H` hours. Returns whether the log changed.
pub fn apply(log: &mut Log, found: Vec<BurnAlert>, notify: bool, now: DateTime<Utc>) -> (Vec<BurnAlert>, bool) {
    let before = log.len();
    let recent = |t: &Option<DateTime<Utc>>, h: i64| t.is_some_and(|t| now - t < Duration::hours(h));
    log.retain(|_, e| recent(&e.last.as_ref().map(|a| a.detected_at), KEEP_H) || recent(&e.notified_at, NOTIFY_EVERY_H));
    let mut changed = log.len() != before;
    let mut due = vec![];
    for alert in found {
        let entry = log.entry(key(alert.provider, &alert.id)).or_default();
        if notify && !recent(&entry.notified_at, NOTIFY_EVERY_H) {
            entry.notified_at = Some(now);
            due.push(alert.clone());
        }
        entry.last = Some(alert);
        changed = true;
    }
    (due, changed)
}

/// What was seen in the past day, newest first.
pub fn seen(log: &Log, now: DateTime<Utc>) -> Vec<BurnAlert> {
    let mut list: Vec<BurnAlert> = log.values().filter_map(|e| e.last.clone()).filter(|a| now - a.detected_at < Duration::hours(KEEP_H)).collect();
    list.sort_by(|a, b| b.detected_at.cmp(&a.detected_at).then_with(|| a.id.cmp(&b.id)));
    list
}

fn round(v: f64) -> f64 { (v * 10.0).round() / 10.0 }

fn path() -> Option<std::path::PathBuf> {
    crate::settings::app_dir().map(|d| d.join("burn-alerts.json"))
}

pub fn load() -> Log {
    path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(log: &Log) -> Result<(), String> {
    let p = path().ok_or("no config dir")?;
    let bytes = serde_json::to_vec(log).map_err(|e| e.to_string())?;
    crate::private_store::write(&p, &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_alerts::tests::{account, at};

    const NOW: &str = "2026-10-21T12:00:00Z";

    /// A Codex account read at `minute` (relative to NOW) at `used` percent of its week.
    fn reading(id: &str, used: f64, minute: i64) -> AccountUsage {
        let now = at(NOW);
        let mut a = account(id, used, now + Duration::days(3));
        a.fetched_at = Some(now + Duration::minutes(minute));
        a
    }

    /// Polls at each of `minutes`, every account at its share then; returns the last readings.
    fn feed(recent: &mut Recent, minutes: &[i64], series: &[(&str, &[f64])]) -> Vec<AccountUsage> {
        let mut last = vec![];
        for (i, m) in minutes.iter().enumerate() {
            last = series.iter().map(|(id, used)| reading(id, used[i], *m)).collect();
            recent.record(&last, at(NOW) + Duration::minutes(*m));
        }
        last
    }

    fn rate(id: &str, repo: &str, recent: u64, baseline: Option<f64>, windows: u32) -> LocalRate {
        LocalRate { provider: Provider::Codex, id: Some(id.into()), repo: repo.into(), model: "gpt-5.5-codex".into(), recent, baseline, active_windows: windows }
    }

    #[test]
    fn a_weekly_rise_past_the_threshold_is_flagged_with_the_repo_behind_it_and_when_it_runs_out() {
        let mut recent = Recent::default();
        // 20 points in 50 minutes (24 points an hour), and 10 in the same time: under the default 15.
        let list = feed(&mut recent, &[-50, -25, 0], &[("fast", &[30.0, 40.0, 50.0]), ("calm", &[30.0, 35.0, 40.0])]);
        let rates = [rate("fast", "api", 9_000_000, Some(8_000_000.0), 10), rate("fast", "web", 1_000, None, 0)];
        let found = detect(&list, &recent, &rates, &Settings::default(), at(NOW));
        assert_eq!(found.len(), 1);
        let a = &found[0];
        assert_eq!((a.id.as_str(), a.trigger, a.rise_points, a.rise_minutes, a.pace_per_hour), ("fast", Trigger::WeeklyRise, Some(20.0), Some(50), Some(24.0)));
        assert_eq!(a.email.as_deref(), Some("fast@example.invalid"));
        assert_eq!(a.this_pc.as_ref().map(|c| (c.repo.as_str(), c.model.as_str(), c.tokens)), Some(("api", "gpt-5.5-codex", 9_000_000)));
        // 49.5 points left at 24 an hour: 2 h 3 min 45 s.
        assert_eq!(a.limit_at, Some(at(NOW) + Duration::seconds(7425)));

        // A lower threshold, or a longer window that still holds only these readings, catches the calm one too.
        let mut settings = Settings::default();
        settings.burn.points = 10;
        assert_eq!(detect(&[reading("calm", 40.0, 0)], &recent, &[], &settings, at(NOW)).len(), 1);
        // A shorter window sees only the last 25 minutes of the fast one: 10 points.
        settings.burn = crate::settings::Burn { points: 15, minutes: 30, token_factor: 3 };
        assert!(detect(&[reading("fast", 50.0, 0)], &recent, &rates, &settings, at(NOW)).is_empty());
    }

    #[test]
    fn a_repo_far_above_its_own_baseline_is_flagged_but_not_a_new_or_small_one() {
        let mut recent = Recent::default();
        let acc = feed(&mut recent, &[-60, 0], &[("a", &[10.0, 14.0])]).remove(0);
        let flagged = |rates: &[LocalRate]| detect(std::slice::from_ref(&acc), &recent, rates, &Settings::default(), at(NOW));
        let found = flagged(&[rate("a", "api", 12_000_000, Some(3_000_000.0), 20)]);
        assert_eq!(found.len(), 1);
        let a = &found[0];
        assert_eq!((a.trigger, a.rise_points), (Trigger::LocalRate, None));
        assert_eq!(a.this_pc, Some(Culprit { repo: "api".into(), model: "gpt-5.5-codex".into(), tokens: 12_000_000, baseline: Some(3_000_000.0) }));
        // 4 points an hour from the trail; 85.5 left: 21 h 22.5 min, before the reset in three days.
        assert_eq!(a.pace_per_hour, Some(4.0));
        assert_eq!(a.limit_at, Some(at(NOW) + Duration::seconds(76950)));
        assert!(flagged(&[rate("a", "api", 8_000_000, Some(3_000_000.0), 20)]).is_empty(), "under three times its usual");
        assert!(flagged(&[rate("a", "new", 50_000_000, None, 0)]).is_empty(), "no history");
        assert!(flagged(&[rate("a", "thin", 50_000_000, Some(1_000.0), 2)]).is_empty(), "too little history");
        assert!(flagged(&[rate("a", "tiny", 1_000_000, Some(1_000.0), 20)]).is_empty(), "too few tokens to matter");
        assert!(flagged(&[rate("b", "api", 12_000_000, Some(3_000_000.0), 20)]).is_empty(), "another account's repo");
        // Stale or failed readings are never judged.
        let mut stale = acc.clone();
        stale.stale = true;
        assert!(detect(&[stale], &recent, &[rate("a", "api", 12_000_000, Some(3_000_000.0), 20)], &Settings::default(), at(NOW)).is_empty());
    }

    #[test]
    fn notifications_are_rate_limited_per_account_and_respect_the_toggle() {
        let now = at(NOW);
        let alert = |id: &str, minute: i64| BurnAlert {
            provider: Provider::Codex, id: id.into(), email: None, alias: None, trigger: Trigger::WeeklyRise, detected_at: now + Duration::minutes(minute),
            rise_points: Some(20.0), rise_minutes: Some(60), used_percent: Some(50.0), pace_per_hour: Some(20.0), limit_at: None, resets_at: None, this_pc: None,
        };
        let mut log = Log::new();
        let (due, changed) = apply(&mut log, vec![alert("a", 0), alert("b", 0)], true, now);
        assert_eq!((due.len(), changed), (2, true));
        let (due, _) = apply(&mut log, vec![alert("a", 60)], true, now + Duration::minutes(60));
        assert!(due.is_empty(), "within three hours");
        assert_eq!(seen(&log, now + Duration::minutes(60))[0].detected_at, now + Duration::minutes(60), "still logged, newest first");
        let (due, _) = apply(&mut log, vec![alert("a", 181)], true, now + Duration::minutes(181));
        assert_eq!(due.len(), 1);
        // Off: logged for agents, never due.
        let mut quiet = Log::new();
        let (due, changed) = apply(&mut quiet, vec![alert("a", 0)], false, now);
        assert!(due.is_empty() && changed && seen(&quiet, now).len() == 1);
        // A day after the last one everything is forgotten.
        let later = now + Duration::hours(28);
        let (_, changed) = apply(&mut log, vec![], true, later);
        assert!(changed && log.is_empty() && seen(&log, later).is_empty());
    }
}
