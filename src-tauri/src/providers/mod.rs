pub mod claude;
pub mod codex;
pub mod evidence;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::monitoring::{self, Monitor};

use crate::accounts::{Active, Provider, Slot};
use crate::model::{AccountUsage, ResetGrant, UsageWindow};
use crate::settings::Settings;
use crate::diagnostics::{self, Code, Event, Origin};

/// Last successful numbers per account, used when its saved token can't be used or the usage
/// API asks us to slow down.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Cached {
    windows: Vec<UsageWindow>,
    credits: Option<String>,
    resets: Vec<ResetGrant>,
    at: Option<DateTime<Utc>>,
    /// Don't call the usage API for this account before this time (after a 429).
    blocked_until: Option<DateTime<Utc>>,
    error: Option<String>,
    monitor_error: Option<String>,
    /// The provider's own billing record, last read at most a few times a day.
    billing: Option<crate::model::BillingHints>,
    #[serde(skip)]
    verified: bool,
}

/// An account is never polled more often than this, however often the app refreshes.
const MIN_INTERVAL: i64 = 60;
/// Wait at least this long after a 429, even if the API asks for less.
const MIN_BACKOFF: i64 = 5 * 60;

/// Error code for a 429, carrying the server's Retry-After (seconds) when present.
pub fn rate_limited(resp: &reqwest::Response) -> String {
    let secs = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    format!("rate_limited:{secs}")
}

pub type Cache = HashMap<String, Cached>;

pub fn retry_now(cache: &mut Cache, key: &str) {
    if let Some(c) = cache.get_mut(key) { c.blocked_until = None; c.verified = false; }
}

pub fn load_cache() -> Cache {
    crate::settings::app_dir().and_then(|d| std::fs::read(d.join("usage-cache.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_cache(cache: &Cache) -> Result<(), String> {
    let dir = crate::settings::app_dir().ok_or("cache_storage")?;
    let bytes = serde_json::to_vec(cache).map_err(|_| "cache_storage")?;
    crate::private_store::write(&dir.join("usage-cache.json"), &bytes).map_err(|_| "cache_storage".into())
}

pub async fn fetch_all(
    client: &reqwest::Client,
    slots: &[Slot],
    active: &Active,
    cache: &mut Cache,
    settings: &Settings,
    monitor: &Monitor,
) -> Vec<AccountUsage> {
    let enabled = |p: Provider| match p {
        Provider::Claude => settings.providers.claude,
        Provider::Codex => settings.providers.codex,
    };
    let slots: Vec<&Slot> = slots.iter().filter(|s| enabled(s.provider)).collect();
    let now = Utc::now();
    // Skip accounts measured moments ago or backing off after a 429: reuse their numbers.
    let skip = |s: &Slot| {
        cache.get(&monitoring::key(s)).is_some_and(|c| {
            c.blocked_until.is_some_and(|t| t > now)
                || (c.verified && c.error.is_none() && c.at.is_some_and(|t| (now - t).num_seconds() < MIN_INTERVAL))
        })
    };
    let results = futures::future::join_all(slots.iter().map(|s| {
        let skipped = skip(s);
        let is_active = active.contains_key(&(s.provider, s.id.clone()));
        async move { if skipped { None } else { Some(fetch_one(client, s, monitor, is_active).await) } }
    }))
    .await;

    let mut out: Vec<AccountUsage> = slots
        .iter()
        .zip(results)
        .map(|(slot, result)| {
            let sources = active.get(&(slot.provider, slot.id.clone())).cloned().unwrap_or_default();
            let mut acc = AccountUsage {
                id: slot.id.clone(),
                provider: slot.provider,
                email: slot.email.clone(),
                alias: settings.aliases.get(&alias_key(slot.provider, &slot.id)).cloned(),
                plan: slot.plan.clone(),
                org: slot.org.clone(),
                active: !sources.is_empty(),
                sources,
                windows: vec![],
                credits: None,
                resets: vec![],
                error: None,
                monitor_error: None,
                stale: false,
                fetched_at: None,
                monitor_connected: monitor.connected(slot),
                billing: slot.billing(),
                remote: vec![],
                used_outside: false,
                runs_out_at: None, unexpected: None, cli_rejected: false,
            };
            let entry = cache.entry(monitoring::key(slot)).or_default();
            match result {
                Some(Ok(Fetched { windows, credits, resets, monitor_error, billing })) => {
                    // Billing is read less often than usage: keep the last record between reads.
                    let billing = match (billing, entry.billing.take()) {
                        (Some(new), Some(old)) => Some(old.overlay(&new)),
                        (new, old) => new.or(old),
                    };
                    *entry = Cached { windows, credits, resets, at: Some(Utc::now()),
                        blocked_until: None, error: None, monitor_error, billing, verified: true };
                }
                Some(Err(err)) => {
                    let backoff = err.strip_prefix("rate_limited:").and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(MIN_BACKOFF).clamp(MIN_BACKOFF, 86400);
                    entry.blocked_until = Some(Utc::now() + chrono::Duration::seconds(backoff));
                    entry.error = Some(if err.starts_with("rate_limited") { "rate_limited".into() } else { err });
                    entry.monitor_error = None;
                    entry.verified = false;
                }
                None => {},
            }
            acc.windows = entry.windows.clone();
            acc.credits = entry.credits.clone();
            acc.resets = entry.resets.clone();
            acc.fetched_at = entry.at;
            acc.stale = !entry.verified || entry.error.is_some();
            acc.error = entry.error.clone();
            acc.monitor_error = entry.monitor_error.clone();
            acc.cli_rejected = cli_rejected(slot);
            if let Some(b) = &entry.billing { acc.billing = acc.billing.clone().overlay(b); }
            if let Some(p) = &acc.billing.plan_type { acc.plan = Some(p.clone()); }
            // The account record names the Pro tiers too, for when usage has not said.
            else if let Some(tier) = match acc.billing.plan_code.as_deref() { Some("chatgptprolite") => Some("prolite"), Some("chatgptpromax") => Some("promax"), _ => None } {
                acc.plan = Some(tier.into());
            }
            acc
        })
        .collect();
    // A stable order: switching accounts must not reshuffle the list.
    out.sort_by_key(|a| (a.provider != Provider::Claude, a.email.clone(), a.id.clone()));
    out
}

/// Whether an account's billing record is due for another read: at most every twelve hours per
/// account and app run, since it changes about once a month.
pub(crate) fn billing_due(key: &str) -> bool {
    use std::time::{Duration, Instant};
    static LAST: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Instant>>> = std::sync::OnceLock::new();
    let mut last = LAST.get_or_init(Default::default).lock().unwrap();
    let due = last.get(key).map_or(true, |t| t.elapsed() >= Duration::from_secs(12 * 3600));
    if due { last.insert(key.to_string(), Instant::now()); }
    due
}

/// CLI logins the provider rejected, per account: a fingerprint of the access token that got the
/// 401. The account counts as rejected only while its saved login still has that token, so a
/// renewal by the CLI clears it without another request.
fn cli_rejections() -> &'static std::sync::Mutex<HashMap<String, u64>> {
    static REJECTED: std::sync::OnceLock<std::sync::Mutex<HashMap<String, u64>>> = std::sync::OnceLock::new();
    REJECTED.get_or_init(Default::default)
}

fn fingerprint(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

/// Notes how a usage read with the CLI's own access token went.
fn note_cli_login(slot: &Slot, token: &str, result: &Result<Fetched, String>) {
    let mut rejected = cli_rejections().lock().unwrap_or_else(|e| e.into_inner());
    match result {
        Err(e) if e == "token_invalid" => { rejected.insert(monitoring::key(slot), fingerprint(token)); }
        Ok(_) => { rejected.remove(&monitoring::key(slot)); }
        Err(_) => {}
    }
}

/// Whether the provider rejected this saved CLI login (see `note_cli_login`).
pub fn cli_rejected(slot: &Slot) -> bool {
    let rejected = cli_rejections().lock().unwrap_or_else(|e| e.into_inner());
    match (rejected.get(&monitoring::key(slot)), slot.access()) {
        (Some(bad), Some((token, _))) => *bad == fingerprint(&token),
        _ => false,
    }
}

pub fn alias_key(provider: Provider, id: &str) -> String {
    format!("{}:{id}", provider.name().to_lowercase())
}

struct Fetched {
    windows: Vec<UsageWindow>,
    credits: Option<String>,
    resets: Vec<ResetGrant>,
    monitor_error: Option<String>,
    billing: Option<crate::model::BillingHints>,
}

trait Usage {
    async fn fetch(&self, provider: Provider, token: &str, account: Option<&str>, observe: &evidence::Observer<'_>) -> Result<Fetched, String>;
}
struct Api<'a>(&'a reqwest::Client, &'a Slot);
impl Usage for Api<'_> {
    async fn fetch(&self, provider: Provider, token: &str, account: Option<&str>, observe: &evidence::Observer<'_>) -> Result<Fetched, String> {
        fetch_token(self.0, self.1, provider, token, account, observe).await
    }
}

async fn fetch_one(client: &reqwest::Client, slot: &Slot, monitor: &Monitor, active: bool) -> Result<Fetched, String> {
    fetch_one_with(client, slot, monitor, active, &Api(client, slot)).await
}

async fn fetch_one_with(client: &reqwest::Client, slot: &Slot, monitor: &Monitor, active: bool, usage: &impl Usage) -> Result<Fetched, String> {
    let result = fetch_session(client, slot, monitor, usage).await;
    // Only the currently captured CLI login may supply a fallback access token.
    // Read usage once; never renew or revoke that login, and never bypass a 429.
    match result {
        Err(error) if active && monitor.connected(slot) && !slot.expired() && !error.starts_with("rate_limited") => {
            if let Some((token, account)) = slot.access() {
                match observed_usage(usage, monitor, slot, &token, account.as_deref(), Origin::ActiveCliFallback).await {
                    Ok(mut live) => {
                    // Fresh telemetry stays eligible for balancing. Keep the monitor's
                    // separate error visible so its Reconnect control is still honest.
                    live.monitor_error = Some(error);
                    return Ok(live);
                    }
                    Err(error) if error.starts_with("rate_limited") => return Err(error),
                    Err(_) => {},
                }
            }
            Err(error)
        }
        result => result,
    }
}

async fn fetch_session(client: &reqwest::Client, slot: &Slot, monitor: &Monitor, usage: &impl Usage) -> Result<Fetched, String> {
    let owned = monitor.connected(slot);
    let (token, account) = if owned {
        monitor.token(slot, client, None).await?
    } else {
        if slot.expired() { return Err("monitor_setup".into()); }
        slot.access().ok_or("monitor_setup")?
    };
    let origin = if owned { Origin::Monitoring } else { Origin::Cli };
    match observed_usage(usage, monitor, slot, &token, account.as_deref(), origin).await {
        Err(e) if e == "token_invalid" && owned => {
            let (renewed, account) = monitor.token(slot, client, Some(&token)).await?;
            // Exactly one renewal and one usage retry for the independent session.
            match observed_usage(usage, monitor, slot, &renewed, account.as_deref(), Origin::Monitoring).await {
                Err(e) if e == "token_invalid" => {
                    monitor.reject(slot, &renewed).await?;
                    Err("monitor_reconnect".into())
                }
                result => result,
            }
        }
        Err(e) if e == "token_invalid" => Err("monitor_setup".into()),
        result => result,
    }
}

async fn observed_usage(usage: &impl Usage, monitor: &Monitor, slot: &Slot, token: &str, account: Option<&str>, origin: Origin) -> Result<Fetched, String> {
    let started = std::time::Instant::now();
    let observe = |evidence| monitor.record(slot, Event::ProviderEvidence { origin, evidence });
    let result = usage.fetch(slot.provider, token, account, &observe).await;
    if matches!(origin, Origin::Cli | Origin::ActiveCliFallback) {
        note_cli_login(slot, token, &result);
    }
    monitor.record(slot, Event::Usage { origin, code: Code::from_error(result.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
    result
}

async fn fetch_token(client: &reqwest::Client, slot: &Slot, provider: Provider, token: &str, account: Option<&str>, observe: &evidence::Observer<'_>) -> Result<Fetched, String> {
    match provider {
        Provider::Claude => claude::fetch(client, token, slot, observe)
            .await.map(|u| Fetched { windows: u.windows, credits: None, resets: u.resets, monitor_error: None, billing: u.billing }),
        Provider::Codex => codex::fetch(client, token, account, observe)
            .await.map(|u| Fetched { windows: u.windows, credits: u.credits, resets: u.resets, monitor_error: None, billing: u.billing }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{slot, TempDir};

    #[tokio::test]
    async fn repeated_monitor_usage_failure_and_cli_fallback_have_independent_diagnostics() {
        struct Observed(std::sync::Mutex<Vec<String>>);
        impl Usage for Observed {
            async fn fetch(&self, _: Provider, token: &str, _: Option<&str>, _: &evidence::Observer<'_>) -> Result<Fetched, String> {
                self.0.lock().unwrap().push(token.to_owned());
                if token == "monitor-access" { return Err("bad_response".into()); }
                Ok(Fetched { windows: observed(24.0).windows, credits: None, resets: vec![], monitor_error: None, billing: None })
            }
        }
        let dir = TempDir::new();
        let recorder = std::sync::Arc::new(crate::diagnostics::Recorder::new(dir.0.join("journal")));
        recorder.start();
        let monitor = Monitor::new(dir.0.join("monitoring")).with_diagnostics(recorder.clone());
        let account = slot("active:org", "live-cli-access", "cli-owned-refresh", i64::MAX);
        let login = slot("active:org", "monitor-access", "monitor-refresh", i64::MAX);
        monitor.connect(&account, &login, &[]).await.unwrap();
        let usage = Observed(std::sync::Mutex::new(vec![]));
        let client = reqwest::Client::new();
        let original = serde_json::to_vec(&account).unwrap();
        for _ in 0..60 {
            let result = fetch_one_with(&client, &account, &monitor, true, &usage).await.unwrap();
            assert_eq!(result.windows[0].used_percent, 24.0);
            assert_eq!(result.monitor_error.as_deref(), Some("bad_response"));
        }
        let calls = usage.0.lock().unwrap();
        assert_eq!(calls.len(), 120);
        for pair in calls.chunks_exact(2) { assert_eq!(pair, ["monitor-access", "live-cli-access"]); }
        assert_eq!(serde_json::to_vec(&account).unwrap(), original);
        let report: serde_json::Value = serde_json::from_slice(&recorder.report(&crate::model::Snapshot::default(), &Settings::default()).unwrap()).unwrap();
        let events = report["events"].as_array().unwrap();
        let readings: Vec<_> = events.iter().filter(|r| r["event"]["type"] == "usage").collect();
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0]["event"]["origin"], "monitoring");
        assert_eq!(readings[0]["event"]["code"], "bad_response");
        assert_eq!(readings[1]["event"]["origin"], "active_cli_fallback");
        assert_eq!(readings[1]["event"]["code"], "ok");
        assert!(!events.iter().any(|r| r["event"]["type"] == "refresh"));
    }

    #[tokio::test]
    async fn only_active_cli_access_tokens_can_supply_fresh_usage_after_monitor_failure() {
        struct Observed(std::sync::Mutex<Vec<String>>);
        impl Usage for Observed {
            async fn fetch(&self, _: Provider, token: &str, _: Option<&str>, _: &evidence::Observer<'_>) -> Result<Fetched, String> {
                self.0.lock().unwrap().push(token.to_owned());
                Ok(Fetched { windows: observed(24.0).windows, credits: None, resets: vec![], monitor_error: None, billing: None })
            }
        }
        for active in [false, true] {
            let dir = TempDir::new();
            let recorder = std::sync::Arc::new(crate::diagnostics::Recorder::new(dir.0.join("journal")));
            recorder.start();
            let monitor = Monitor::new(dir.0.join("monitoring")).with_diagnostics(recorder.clone());
            let account = slot("active:org", "live-cli-access", "cli-owned-refresh", i64::MAX);
            let login = slot("active:org", "monitor-access", "monitor-refresh", i64::MAX);
            monitor.connect(&account, &login, &[]).await.unwrap();
            // Persist a rejected monitor, so any attempted automatic renewal fails
            // the test rather than hitting a real provider.
            monitor.token(&account, &reqwest::Client::new(), None).await.unwrap();
            monitor.reject(&account, "monitor-access").await.unwrap();
            let usage = Observed(std::sync::Mutex::new(vec![]));
            let original = serde_json::to_vec(&account).unwrap();
            let result = fetch_one_with(&reqwest::Client::new(), &account, &monitor, active, &usage).await;
            if active {
                let fetched = result.unwrap();
                assert_eq!(fetched.windows[0].used_percent, 24.0);
                assert_eq!(fetched.monitor_error.as_deref(), Some("monitor_reconnect"));
                assert_eq!(*usage.0.lock().unwrap(), ["live-cli-access"]);
            } else {
                assert_eq!(result.err().as_deref(), Some("monitor_reconnect"));
                assert!(usage.0.lock().unwrap().is_empty());
            }
            assert_eq!(serde_json::to_vec(&account).unwrap(), original);
            let report = String::from_utf8(recorder.report(&crate::model::Snapshot::default(), &Settings::default()).unwrap()).unwrap();
            for private in ["live-cli-access", "cli-owned-refresh", "monitor-access", "monitor-refresh", "active:org"] { assert!(!report.contains(private)); }
            assert_eq!(report.contains("active_cli_fallback"), active);
        }
    }

    fn observed(percent: f64) -> Cached {
        Cached {
            windows: vec![UsageWindow { kind: "session".into(), window_seconds: Some(18000), used_percent: percent,
                resets_at: Some(Utc::now() - chrono::Duration::hours(1)) }],
            at: Some(Utc::now() - chrono::Duration::hours(2)), verified: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn expired_imported_cli_session_requests_setup_without_refreshing_or_network_calls() {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        let account = slot("inactive:org", "expired", "cli-owned-refresh", 0);
        let mut cache = Cache::new();
        let results = fetch_all(&reqwest::Client::new(), &[account], &Active::new(), &mut cache, &Settings::default(), &monitor).await;
        assert_eq!(results[0].error.as_deref(), Some("monitor_setup"));
        assert!(results[0].stale);
        assert!(results[0].windows.is_empty());
        assert!(!results[0].monitor_connected);
        assert!(std::fs::read_dir(&dir.0).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn restart_and_expired_reset_keep_last_observed_usage_instead_of_inventing_zero() {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        let account = slot("inactive:org", "expired", "cli-owned-refresh", 0);
        let original = observed(87.0);
        let measured_at = original.at;
        let original_reset = original.windows[0].resets_at;
        let mut before = Cache::new();
        before.insert(monitoring::key(&account), original);
        let bytes = serde_json::to_vec(&before).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("refresh"));
        let mut restarted: Cache = serde_json::from_slice(&bytes).unwrap();
        assert!(!restarted[&monitoring::key(&account)].verified);
        let results = fetch_all(&reqwest::Client::new(), &[account], &Active::new(), &mut restarted, &Settings::default(), &monitor).await;
        assert!(results[0].stale);
        assert_eq!(results[0].windows[0].used_percent, 87.0);
        assert_eq!(results[0].windows[0].resets_at, original_reset);
        assert_eq!(results[0].fetched_at, measured_at);
    }

    #[tokio::test]
    async fn cache_is_provider_scoped_and_backoff_preserves_error_and_timestamp() {
        let dir = TempDir::new();
        let monitor = Monitor::new(dir.0.clone());
        let claude = slot("same-id", "unused", "unused", 0);
        let mut codex = claude.clone();
        codex.provider = Provider::Codex;
        let mut cache = Cache::new();
        for (account, percent) in [(&claude, 87.0), (&codex, 12.0)] {
            let mut entry = observed(percent);
            entry.error = Some("rate_limited".into());
            entry.blocked_until = Some(Utc::now() + chrono::Duration::hours(1));
            cache.insert(monitoring::key(account), entry);
        }
        let results = fetch_all(&reqwest::Client::new(), &[claude, codex], &Active::new(), &mut cache, &Settings::default(), &monitor).await;
        assert_eq!(results[0].windows[0].used_percent, 87.0);
        assert_eq!(results[1].windows[0].used_percent, 12.0);
        assert!(results.iter().all(|a| a.stale && a.error.as_deref() == Some("rate_limited")));
        assert_eq!(cache.len(), 2);
    }
}
