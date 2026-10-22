//! When each account was the one signed in to its provider's CLI on this machine, as intervals.
//! Claude Code's transcripts name no account, so the usage breakdown (`usage_index`) attributes
//! a Claude message to the account that was active when it was written. Time SlopBar did not
//! see (the app closed, the PC asleep) is a gap: nothing in it is attributed.
//!
//! Only slot ids and times are kept, never emails.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;

const VERSION: u8 = 1;
/// Intervals older than this are dropped.
const KEEP_DAYS: i64 = 90;
const MAX_FILE: u64 = 4 << 20;
/// An interval is extended by a reading at most this long after its end; a longer silence
/// starts a new one (see `max_gap`).
pub const MIN_GAP_SECONDS: i64 = 15 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    /// Vault slot id.
    pub id: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Log {
    pub version: u8,
    /// Keyed by `Provider::key`, oldest first, never overlapping.
    pub providers: HashMap<String, Vec<Interval>>,
    /// The diagnostics journal was folded in once (see `backfill`).
    pub backfilled: bool,
}

/// The longest silence that still counts as the same stretch of use: three refreshes, at least
/// a quarter of an hour.
pub fn max_gap(refresh_minutes: u32) -> Duration {
    Duration::seconds((3 * 60 * refresh_minutes.max(1) as i64 + 60).max(MIN_GAP_SECONDS))
}

impl Log {
    /// Records that `id` was the active account of `provider` at `now`. Returns whether a new
    /// interval began (worth saving at once).
    pub fn note(&mut self, provider: Provider, id: &str, now: DateTime<Utc>, gap: Duration) -> bool {
        let list = self.providers.entry(provider.key().into()).or_default();
        if let Some(last) = list.last_mut() {
            if now < last.to { return false; }
            if last.id == id && now - last.to <= gap {
                last.to = now;
                return false;
            }
        }
        list.push(Interval { id: id.into(), from: now, to: now });
        true
    }

    /// The account active for `provider` at `at`, if SlopBar saw one then.
    pub fn at(&self, provider: Provider, at: DateTime<Utc>) -> Option<&str> {
        let list = self.providers.get(provider.key())?;
        let i = list.partition_point(|v| v.from <= at);
        list.get(i.checked_sub(1)?).filter(|v| at <= v.to).map(|v| v.id.as_str())
    }

    pub fn prune(&mut self, now: DateTime<Utc>) {
        let cutoff = now - Duration::days(KEEP_DAYS);
        for list in self.providers.values_mut() { list.retain(|v| v.to >= cutoff); }
    }

    /// Folds in readings from before this log existed (the diagnostics journal keeps about a
    /// week of which account was active): `(time, provider, slot id, active)`, any order. Only
    /// time not already covered here is filled.
    pub fn backfill(&mut self, mut marks: Vec<(DateTime<Utc>, Provider, String, bool)>, gap: Duration) {
        self.backfilled = true;
        marks.sort_by_key(|m| m.0);
        for provider in Provider::ALL {
            let first_own = self.providers.get(provider.key()).and_then(|l| l.first()).map(|v| v.from);
            let mut built = Log::default();
            let mut current: Option<&str> = None;
            for (at, p, id, active) in &marks {
                if *p != provider || first_own.is_some_and(|f| *at >= f) { continue; }
                match (active, current) {
                    (true, _) => { built.note(provider, id, *at, gap); current = Some(id); }
                    // The active account's own reading says it no longer is: its interval ends.
                    (false, Some(c)) if c == id => current = None,
                    // Another account's reading only shows the poll ran: the active one held.
                    (false, Some(c)) => { built.note(provider, c, *at, gap); }
                    (false, None) => {}
                }
            }
            let Some(older) = built.providers.remove(provider.key()) else { continue };
            let list = self.providers.entry(provider.key().into()).or_default();
            let mut merged = older;
            merged.append(list);
            *list = merged;
        }
    }
}

/// The account signed in to each provider's CLI here. Claude Code's transcripts and Codex's
/// rollouts are read from Windows, so when WSL has another account signed in too, the Windows
/// one counts; two with no Windows one between them is too uncertain to record.
pub fn active_ids(accounts: &[crate::model::AccountUsage]) -> Vec<(Provider, &str)> {
    Provider::ALL.into_iter().filter_map(|p| {
        let active: Vec<_> = accounts.iter().filter(|a| a.provider == p && a.active).collect();
        match active.as_slice() {
            [one] => Some((p, one.id.as_str())),
            many => {
                let native: Vec<_> = many.iter().filter(|a| a.sources.iter().any(|s| s == "Windows")).collect();
                (native.len() == 1).then(|| (p, native[0].id.as_str()))
            }
        }
    }).collect()
}

pub fn load() -> Log {
    crate::settings::app_dir().map(|d| load_from(&d.join("active-accounts.json"))).unwrap_or_default()
}

fn load_from(path: &std::path::Path) -> Log {
    let fresh = || Log { version: VERSION, ..Default::default() };
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_FILE) { return fresh(); }
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Log>(&b).ok())
        .filter(|l| l.version == VERSION).unwrap_or_else(fresh)
}

pub fn save(log: &Log) -> Result<(), String> {
    let dir = crate::settings::app_dir().ok_or("history_storage")?;
    save_to(&dir.join("active-accounts.json"), log)
}

fn save_to(path: &std::path::Path, log: &Log) -> Result<(), String> {
    let bytes = serde_json::to_vec(log).map_err(|_| "history_storage")?;
    crate::private_store::write(path, &bytes).map_err(|_| "history_storage".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    #[test]
    fn readings_extend_an_interval_until_a_switch_or_a_gap() {
        let mut log = Log::default();
        let gap = max_gap(5);
        assert!(log.note(Provider::Claude, "a", at("2026-10-01T10:00:00Z"), gap));
        assert!(!log.note(Provider::Claude, "a", at("2026-10-01T10:05:00Z"), gap));
        assert!(!log.note(Provider::Claude, "a", at("2026-10-01T10:20:00Z"), gap));
        // A switch starts the other account's interval at once.
        assert!(log.note(Provider::Claude, "b", at("2026-10-01T10:25:00Z"), gap));
        // Overnight silence: a new interval, even for the same account.
        assert!(log.note(Provider::Claude, "b", at("2026-10-02T08:00:00Z"), gap));
        let list = &log.providers["claude"];
        assert_eq!(list.len(), 3);
        assert_eq!((list[0].from, list[0].to), (at("2026-10-01T10:00:00Z"), at("2026-10-01T10:20:00Z")));
        assert_eq!(log.at(Provider::Claude, at("2026-10-01T10:10:00Z")), Some("a"));
        // Between the last reading of one account and the first of the next nobody is known.
        assert_eq!(log.at(Provider::Claude, at("2026-10-01T10:22:00Z")), None);
        assert_eq!(log.at(Provider::Claude, at("2026-10-01T10:25:00Z")), Some("b"));
        assert_eq!(log.at(Provider::Claude, at("2026-10-01T20:00:00Z")), None, "a gap stays unattributed");
        assert_eq!(log.at(Provider::Codex, at("2026-10-01T10:10:00Z")), None);
        // A reading older than the last one (a clock step back) never rewrites the past.
        assert!(!log.note(Provider::Claude, "a", at("2026-10-01T09:00:00Z"), gap));
    }

    #[test]
    fn backfill_fills_only_time_before_the_log_began() {
        let gap = max_gap(5);
        let mut log = Log::default();
        log.note(Provider::Claude, "b", at("2026-10-05T12:00:00Z"), gap);
        let mark = |s: &str, id: &str, active| (at(s), Provider::Claude, id.to_string(), active);
        log.backfill(vec![
            mark("2026-10-05T09:00:00Z", "a", true),
            mark("2026-10-05T09:10:00Z", "b", false),
            mark("2026-10-05T09:12:00Z", "a", true),
            mark("2026-10-05T09:30:00Z", "a", false),
            mark("2026-10-05T09:40:00Z", "b", false),
            mark("2026-10-05T12:30:00Z", "a", true),
        ], gap);
        assert!(log.backfilled);
        let list = &log.providers["claude"];
        assert_eq!(list.len(), 2, "{list:?}");
        assert_eq!((list[0].id.as_str(), list[0].from, list[0].to), ("a", at("2026-10-05T09:00:00Z"), at("2026-10-05T09:12:00Z")));
        assert_eq!(log.at(Provider::Claude, at("2026-10-05T09:35:00Z")), None, "a's own reading ended it");
        assert_eq!(log.at(Provider::Claude, at("2026-10-05T12:00:00Z")), Some("b"));
        assert_eq!(log.at(Provider::Claude, at("2026-10-05T12:30:00Z")), None, "the journal never overrides the live log");
    }

    #[test]
    fn survives_a_restart_and_prunes_old_intervals() {
        let dir = crate::test_support::TempDir::new();
        let path = dir.0.join("active-accounts.json");
        let mut log = Log { version: VERSION, ..Default::default() };
        log.note(Provider::Codex, "user:acct", at("2026-06-01T00:00:00Z"), max_gap(5));
        log.note(Provider::Codex, "user:other", at("2026-10-01T00:00:00Z"), max_gap(5));
        log.prune(at("2026-10-02T00:00:00Z"));
        save_to(&path, &log).unwrap();
        let back = load_from(&path);
        assert_eq!(back.providers["codex"].len(), 1);
        assert_eq!(back.at(Provider::Codex, at("2026-10-01T00:00:00Z")), Some("user:other"));
        std::fs::write(&path, b"{broken").unwrap();
        assert!(load_from(&path).providers.is_empty());
    }
}
