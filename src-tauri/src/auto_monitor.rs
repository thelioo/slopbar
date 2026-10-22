//! Connect monitoring automatically: once an account without a monitoring sign-in has been the
//! one in use for a few minutes, start the same sign-in as Settings' Connect monitoring. While an
//! account is in use its usage is read with the CLI's own token, so this is the moment the user's
//! browser is most likely signed in to it; after a switch away it would go dark.
use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;
use crate::model::AccountUsage;

const VERSION: u8 = 1;
/// How long an account must stay in use before its sign-in starts on its own.
pub const ACTIVE_FOR: Duration = Duration::minutes(3);
/// After a cancelled, timed-out or failed automatic sign-in, the account is left alone this long.
pub const BACKOFF: Duration = Duration::hours(24);

/// What the trigger needs to know about one account, from the last snapshot.
#[derive(Debug, Clone)]
pub struct Seen {
    pub provider: Provider,
    pub id: String,
    pub active: bool,
    pub connected: bool,
}

impl Seen {
    pub fn of(a: &AccountUsage) -> Self {
        Self { provider: a.provider, id: a.id.clone(), active: a.active, connected: a.monitor_connected }
    }
    fn key(&self) -> String {
        format!("{}:{}", self.provider.name(), self.id)
    }
    /// A slot known only by a token hash: no email or account id to check the sign-in against.
    fn token_only(&self) -> bool {
        self.id.starts_with("token:")
    }
}

/// Persisted between runs, so a restart never asks again during a backoff.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Saved {
    pub version: u8,
    /// Keyed like `monitoring::key`: no automatic sign-in for the account before this time.
    #[serde(default)]
    pub backoff: HashMap<String, DateTime<Utc>>,
    /// Accounts whose monitoring the user disconnected: never connected automatically again,
    /// until the user connects them by hand.
    #[serde(default)]
    pub declined: HashSet<String>,
}

/// The trigger's state: `saved` plus, in memory only, since when each unmonitored account has
/// been in use (a restart starts the wait over).
#[derive(Debug, Default)]
pub struct Tracker {
    pub saved: Saved,
    since: HashMap<String, DateTime<Utc>>,
    /// The account whose automatic sign-in is running; one at a time across providers.
    running: Option<String>,
}

/// Everything outside the snapshot that can hold a sign-in back.
pub struct Guards<'a> {
    /// Settings: Connect monitoring automatically.
    pub enabled: bool,
    /// A fullscreen app (video, game, presentation) is in front: wait rather than pop a window.
    pub fullscreen: bool,
    /// Accounts with a connect, disconnect or removal in progress (`monitoring::Operations`).
    pub busy: &'a dyn Fn(&str) -> bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Start the monitoring sign-in for this account now.
    Start { provider: Provider, id: String },
    /// An account is due but a fullscreen app is in front; asked again at the next tick.
    Deferred,
    Nothing,
}

impl Tracker {
    pub fn new(saved: Saved) -> Self {
        Self { saved, ..Self::default() }
    }

    /// Updates how long each unmonitored account has been in use and decides whether to start a
    /// sign-in. On `Start` the account counts as running until `finished` is called.
    pub fn decide(&mut self, accounts: &[Seen], guards: &Guards, now: DateTime<Utc>) -> Decision {
        self.saved.backoff.retain(|_, until| *until > now);
        let waiting: HashSet<String> = accounts.iter().filter(|a| a.active && !a.connected).map(Seen::key).collect();
        self.since.retain(|key, _| waiting.contains(key));
        for key in &waiting {
            self.since.entry(key.clone()).or_insert(now);
        }
        if !guards.enabled || self.running.is_some() {
            return Decision::Nothing;
        }
        let due = accounts.iter().find(|a| {
            let key = a.key();
            a.active && !a.connected && !a.token_only()
                && self.since.get(&key).is_some_and(|since| now - *since >= ACTIVE_FOR)
                && !self.saved.backoff.contains_key(&key)
                && !self.saved.declined.contains(&key)
                && !(guards.busy)(&key)
        });
        let Some(due) = due else { return Decision::Nothing };
        if guards.fullscreen {
            return Decision::Deferred;
        }
        self.running = Some(due.key());
        Decision::Start { provider: due.provider, id: due.id.clone() }
    }

    /// The automatic sign-in for `key` ended. Anything but success backs off for a day. Returns
    /// whether `saved` changed.
    pub fn finished(&mut self, key: &str, ok: bool, now: DateTime<Utc>) -> bool {
        if self.running.as_deref() == Some(key) {
            self.running = None;
        }
        self.since.remove(key);
        if ok {
            self.saved.backoff.remove(key).is_some()
        } else {
            self.saved.backoff.insert(key.to_string(), now + BACKOFF);
            true
        }
    }

    /// The user connected (`connected`) or disconnected monitoring by hand. A disconnect is a
    /// choice to leave the account unmonitored; a connect undoes it. Returns whether `saved` changed.
    pub fn manual(&mut self, key: &str, connected: bool) -> bool {
        if connected {
            let backoff = self.saved.backoff.remove(key).is_some();
            self.saved.declined.remove(key) || backoff
        } else {
            self.since.remove(key);
            self.saved.declined.insert(key.to_string())
        }
    }
}

pub fn load() -> Saved {
    crate::settings::app_dir().map(|d| load_from(&d.join("monitor-auto.json"))).unwrap_or_default()
}

fn load_from(path: &std::path::Path) -> Saved {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
        .filter(|s| s.version == VERSION).unwrap_or(Saved { version: VERSION, ..Default::default() })
}

pub fn save(saved: &Saved) -> Result<(), String> {
    let dir = crate::settings::app_dir().ok_or("no config dir")?;
    save_to(&dir.join("monitor-auto.json"), saved)
}

fn save_to(path: &std::path::Path, saved: &Saved) -> Result<(), String> {
    let mut saved = saved.clone();
    saved.version = VERSION;
    let bytes = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
    crate::private_store::write(path, &bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-05T12:00:00Z").unwrap().with_timezone(&Utc) + Duration::minutes(minutes)
    }

    fn seen(provider: Provider, id: &str, active: bool, connected: bool) -> Seen {
        Seen { provider, id: id.into(), active, connected }
    }

    const IDLE: &dyn Fn(&str) -> bool = &|_| false;

    fn guards(enabled: bool, fullscreen: bool) -> Guards<'static> {
        Guards { enabled, fullscreen, busy: IDLE }
    }

    fn start(id: &str) -> Decision {
        Decision::Start { provider: Provider::Claude, id: id.into() }
    }

    #[test]
    fn starts_after_three_minutes_in_use() {
        let mut t = Tracker::default();
        let list = [seen(Provider::Claude, "a", true, false)];
        assert_eq!(t.decide(&list, &guards(true, false), at(0)), Decision::Nothing);
        assert_eq!(t.decide(&list, &guards(true, false), at(2)), Decision::Nothing);
        assert_eq!(t.decide(&list, &guards(true, false), at(3)), start("a"));
    }

    #[test]
    fn switching_away_restarts_the_wait() {
        let mut t = Tracker::default();
        let on = [seen(Provider::Claude, "a", true, false)];
        let off = [seen(Provider::Claude, "a", false, false)];
        t.decide(&on, &guards(true, false), at(0));
        t.decide(&off, &guards(true, false), at(2));
        assert_eq!(t.decide(&on, &guards(true, false), at(4)), Decision::Nothing, "the clock restarted at minute 4");
        assert_eq!(t.decide(&on, &guards(true, false), at(7)), start("a"));
    }

    #[test]
    fn never_for_connected_inactive_or_token_only_accounts() {
        let mut t = Tracker::default();
        let list = [
            seen(Provider::Claude, "linked", true, true),
            seen(Provider::Claude, "idle", false, false),
            seen(Provider::Codex, "token:abc123", true, false),
        ];
        t.decide(&list, &guards(true, false), at(0));
        assert_eq!(t.decide(&list, &guards(true, false), at(60)), Decision::Nothing);
    }

    #[test]
    fn setting_off_starts_nothing() {
        let mut t = Tracker::default();
        let list = [seen(Provider::Claude, "a", true, false)];
        t.decide(&list, &guards(false, false), at(0));
        assert_eq!(t.decide(&list, &guards(false, false), at(10)), Decision::Nothing);
        assert_eq!(t.decide(&list, &guards(true, false), at(10)), start("a"), "turning it on starts at once: the account was in use all along");
    }

    #[test]
    fn fullscreen_defers_until_it_ends() {
        let mut t = Tracker::default();
        let list = [seen(Provider::Claude, "a", true, false)];
        t.decide(&list, &guards(true, true), at(0));
        assert_eq!(t.decide(&list, &guards(true, true), at(5)), Decision::Deferred);
        assert_eq!(t.decide(&list, &guards(true, true), at(50)), Decision::Deferred);
        assert_eq!(t.decide(&list, &guards(true, false), at(51)), start("a"));
    }

    #[test]
    fn pending_sign_ins_block_their_account() {
        let mut t = Tracker::default();
        let list = [seen(Provider::Claude, "a", true, false)];
        let busy: &dyn Fn(&str) -> bool = &|key| key == "Claude:a";
        t.decide(&list, &Guards { enabled: true, fullscreen: false, busy }, at(0));
        assert_eq!(t.decide(&list, &Guards { enabled: true, fullscreen: false, busy }, at(5)), Decision::Nothing);
    }

    #[test]
    fn one_automatic_sign_in_at_a_time_across_providers() {
        let mut t = Tracker::default();
        let list = [seen(Provider::Claude, "a", true, false), seen(Provider::Codex, "b", true, false)];
        t.decide(&list, &guards(true, false), at(0));
        assert_eq!(t.decide(&list, &guards(true, false), at(3)), start("a"));
        assert_eq!(t.decide(&list, &guards(true, false), at(4)), Decision::Nothing, "a is still running");
        assert!(!t.finished("Claude:a", true, at(5)));
        let after = [seen(Provider::Claude, "a", true, true), seen(Provider::Codex, "b", true, false)];
        assert_eq!(t.decide(&after, &guards(true, false), at(5)), Decision::Start { provider: Provider::Codex, id: "b".into() });
    }

    #[test]
    fn a_failed_sign_in_backs_off_for_a_day() {
        let mut t = Tracker::default();
        let list = [seen(Provider::Claude, "a", true, false)];
        t.decide(&list, &guards(true, false), at(0));
        assert_eq!(t.decide(&list, &guards(true, false), at(3)), start("a"));
        assert!(t.finished("Claude:a", false, at(10)), "the backoff is saved");
        assert_eq!(t.saved.backoff["Claude:a"], at(10) + BACKOFF);
        assert_eq!(t.decide(&list, &guards(true, false), at(20)), Decision::Nothing);
        assert_eq!(t.decide(&list, &guards(true, false), at(10 + 24 * 60 - 1)), Decision::Nothing);
        assert_eq!(t.decide(&list, &guards(true, false), at(10 + 24 * 60)), start("a"));
        assert!(t.saved.backoff.is_empty(), "expired backoffs are forgotten");
    }

    #[test]
    fn a_saved_backoff_survives_a_restart() {
        let dir = crate::test_support::TempDir::new();
        let path = dir.0.join("monitor-auto.json");
        let mut t = Tracker::default();
        t.finished("Claude:a", false, at(0));
        save_to(&path, &t.saved).unwrap();
        let mut t = Tracker::new(load_from(&path));
        let list = [seen(Provider::Claude, "a", true, false)];
        t.decide(&list, &guards(true, false), at(1));
        assert_eq!(t.decide(&list, &guards(true, false), at(30)), Decision::Nothing);
        assert!(load_from(&dir.0.join("missing.json")).backoff.is_empty());
    }

    #[test]
    fn a_manual_disconnect_is_respected_until_a_manual_connect() {
        let mut t = Tracker::default();
        let list = [seen(Provider::Claude, "a", true, false)];
        assert!(t.manual("Claude:a", false));
        t.decide(&list, &guards(true, false), at(0));
        assert_eq!(t.decide(&list, &guards(true, false), at(30)), Decision::Nothing);
        assert!(t.manual("Claude:a", true));
        assert_eq!(t.decide(&list, &guards(true, false), at(31)), start("a"));
    }
}
