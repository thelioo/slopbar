//! Unexpected use: with in-use sharing on, every computer of the user's that runs SlopBar says
//! which accounts are signed in there. A monitored account whose weekly usage jumps while none
//! of them claims it (see `presence::infer`) was used somewhere else: the web or mobile app, or
//! someone else holding the login. Each occasion is logged for a week, and the user is told at
//! most once every six hours per account. SlopBar only points at the provider's own security
//! page; signing other sessions out is left to the user.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::model::{AccountUsage, UnexpectedUse};

/// At most one notification per account in this many hours.
const NOTIFY_EVERY_H: i64 = 6;
/// Surges this close to the previous one are the same occasion (one long web session).
const OCCASION_MIN: i64 = 60;
const KEEP_DAYS: i64 = 7;

/// One account's occasions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// When each occasion started, within the last week.
    pub seen: Vec<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
    pub notified_at: Option<DateTime<Utc>>,
}

/// Keyed like the settings' `aliases` ("provider:account id").
pub type Log = HashMap<String, Entry>;

/// What decides whether surges count.
pub struct Watch<'a> {
    /// Sharing is on and this machine or a peer published recently: the user has opted into
    /// covering every computer, so use no computer claims is worth reporting.
    pub covered: bool,
    /// The notification is turned on.
    pub notify: bool,
    /// Accounts also used on the web or a phone (`Settings::web_use`).
    pub web_use: &'a HashMap<String, bool>,
}

pub fn key(provider: Provider, id: &str) -> String {
    format!("{}:{}", provider.key(), id)
}

/// Logs this reading's surges and fills each account's `unexpected`. Returns whether the log
/// changed (and should be saved).
pub fn apply(log: &mut Log, accounts: &mut [AccountUsage], surged: &[(Provider, String)], watch: &Watch, now: DateTime<Utc>) -> bool {
    let before = log.clone();
    let quiet = |t: &Option<DateTime<Utc>>, within: Duration| !t.is_some_and(|t| now - t < within);
    let web = |k: &str| watch.web_use.get(k).copied().unwrap_or(false);
    for entry in log.values_mut() {
        entry.seen.retain(|t| now - *t < Duration::days(KEEP_DAYS));
    }
    log.retain(|_, e| !e.seen.is_empty() || !quiet(&e.notified_at, Duration::hours(NOTIFY_EVERY_H)));
    if watch.covered {
        for (provider, id) in surged {
            let k = key(*provider, id);
            if web(&k) {
                continue;
            }
            let entry = log.entry(k).or_default();
            if quiet(&entry.last_seen, Duration::minutes(OCCASION_MIN)) || entry.seen.is_empty() {
                entry.seen.push(now);
            }
            entry.last_seen = Some(now);
            if watch.notify && quiet(&entry.notified_at, Duration::hours(NOTIFY_EVERY_H)) {
                entry.notified_at = Some(now);
            }
        }
    }
    for a in accounts.iter_mut() {
        let k = key(a.provider, &a.id);
        a.unexpected = log.get(&k).filter(|e| !e.seen.is_empty() && !web(&k)).map(|e| UnexpectedUse {
            seen_week: e.seen.len() as u32,
            last_seen: e.last_seen.unwrap_or(e.seen[e.seen.len() - 1]),
            alert_at: e.notified_at,
        });
    }
    *log != before
}

fn path() -> Option<std::path::PathBuf> {
    crate::settings::app_dir().map(|d| d.join("unexpected-use.json"))
}

pub fn load() -> Log {
    path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(log: &Log) -> Result<(), String> {
    let p = path().ok_or("no config dir")?;
    let bytes = serde_json::to_vec(log).map_err(|e| e.to_string())?;
    crate::private_store::write(&p, &bytes).map_err(|e| e.to_string())
}

/// The provider's own page for reviewing sessions and devices and changing the password.
pub fn security_page(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "https://claude.ai/settings",
        Provider::Codex => "https://chatgpt.com/#settings/Security",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z").unwrap().with_timezone(&Utc) + Duration::minutes(minutes)
    }

    fn account(id: &str) -> AccountUsage {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "claude", "email": format!("{id}@example.test"), "alias": null, "plan": null, "org": null,
            "active": false, "sources": [], "windows": [], "credits": null, "resets": [], "error": null,
            "stale": false, "fetched_at": null, "monitor_connected": true,
        })).unwrap()
    }

    fn run(log: &mut Log, list: &mut [AccountUsage], surged: &[&str], covered: bool, web_use: &HashMap<String, bool>, minute: i64) -> bool {
        let surged: Vec<_> = surged.iter().map(|id| (Provider::Claude, id.to_string())).collect();
        apply(log, list, &surged, &Watch { covered, notify: true, web_use }, at(minute))
    }

    #[test]
    fn occasions_are_logged_and_notified_at_most_every_six_hours() {
        let none = HashMap::new();
        let mut log = Log::new();
        let mut list = vec![account("a"), account("b")];
        assert!(run(&mut log, &mut list, &["a"], true, &none, 0));
        let first = list[0].unexpected.clone().unwrap();
        assert_eq!((first.seen_week, first.alert_at), (1, Some(at(0))));
        assert!(list[1].unexpected.is_none(), "only the account that surged");

        // Another surge minutes later is the same occasion and no new notification.
        run(&mut log, &mut list, &["a"], true, &none, 30);
        let same = list[0].unexpected.clone().unwrap();
        assert_eq!((same.seen_week, same.last_seen, same.alert_at), (1, at(30), Some(at(0))));

        // A new occasion after a quiet hour is counted, but notified only after six hours.
        run(&mut log, &mut list, &["a"], true, &none, 120);
        assert_eq!(list[0].unexpected.as_ref().map(|u| (u.seen_week, u.alert_at)), Some((2, Some(at(0)))));
        run(&mut log, &mut list, &["a"], true, &none, 6 * 60);
        assert_eq!(list[0].unexpected.as_ref().map(|u| (u.seen_week, u.alert_at)), Some((3, Some(at(360)))));

        // No surge: nothing changes, and the log is shown as it is.
        assert!(!run(&mut log, &mut list, &[], true, &none, 6 * 60 + 5));
        assert_eq!(list[0].unexpected.as_ref().map(|u| u.seen_week), Some(3));

        // Occasions older than a week drop out, and with them the account's entry.
        run(&mut log, &mut list, &[], true, &none, 7 * 24 * 60 + 121);
        assert_eq!(list[0].unexpected.as_ref().map(|u| u.seen_week), Some(1), "only the one from six hours in");
        run(&mut log, &mut list, &[], true, &none, 7 * 24 * 60 + 6 * 60 + 1);
        assert!(list[0].unexpected.is_none() && log.is_empty());
    }

    #[test]
    fn needs_fleet_coverage_and_respects_the_web_opt_out_and_toggle() {
        let none = HashMap::new();
        let mut log = Log::new();
        let mut list = vec![account("a")];
        assert!(!run(&mut log, &mut list, &["a"], false, &none, 0), "sharing off or nobody publishing");
        assert!(list[0].unexpected.is_none());

        let web: HashMap<_, _> = [("claude:a".to_string(), true)].into();
        assert!(!run(&mut log, &mut list, &["a"], true, &web, 0), "also used on the web");
        assert!(list[0].unexpected.is_none());

        // Marking it afterwards hides what was logged.
        run(&mut log, &mut list, &["a"], true, &none, 5);
        assert!(list[0].unexpected.is_some());
        run(&mut log, &mut list, &[], true, &web, 10);
        assert!(list[0].unexpected.is_none());

        // With the notification off, occasions are still logged but none is due.
        let mut log = Log::new();
        apply(&mut log, &mut list, &[(Provider::Claude, "a".into())], &Watch { covered: true, notify: false, web_use: &none }, at(0));
        assert_eq!(list[0].unexpected.as_ref().map(|u| (u.seen_week, u.alert_at)), Some((1, None)));
    }

    #[test]
    fn security_pages_are_the_providers_own() {
        assert_eq!(security_page(Provider::Claude), "https://claude.ai/settings");
        assert_eq!(security_page(Provider::Codex), "https://chatgpt.com/#settings/Security");
    }
}
