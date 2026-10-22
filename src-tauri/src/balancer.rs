//! Automatic account switching: when the signed-in account of a provider reaches the
//! configured share of any usage window, move to the account `headroom::recommend` ranks first
//! among those well below it: the one whose weekly reset or renewal comes soonest, so capacity
//! that would be lost is used first. Accounts in use on another machine, on a plan that is
//! ending or lapsed, or without a fresh reading are never targets.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::accounts::Provider;
use crate::model::AccountUsage;
use crate::settings::Settings;

/// Minimum time between automatic switches of the same provider.
const COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// A candidate must be this far below the threshold, so it doesn't bounce straight back.
const HYSTERESIS: f64 = 10.0;

/// The account to switch to for each provider that needs it.
pub fn decide(
    accounts: &[AccountUsage],
    settings: &Settings,
    last_switch: &HashMap<Provider, Instant>,
) -> Vec<(Provider, String)> {
    let mut out = Vec::new();
    for provider in Provider::ALL {
        let rule = settings.balancer.rule(provider);
        if !rule.auto || last_switch.get(&provider).is_some_and(|t| t.elapsed() < COOLDOWN) {
            continue;
        }
        let of_provider: Vec<&AccountUsage> = accounts.iter().filter(|a| a.provider == provider && !a.stale && a.error.is_none()).collect();
        let Some(current) = of_provider.iter().find(|a| a.active) else { continue };
        let threshold = rule.threshold as f64;
        if current.peak().unwrap_or(0.0) < threshold {
            continue;
        }
        // The recommendation's ranking, limited to measured, current headroom well below the threshold.
        let rec = crate::headroom::recommend(accounts, settings, provider, None, None, chrono::Utc::now());
        let best = rec.ranking.iter()
            .filter(|c| c.fits && !c.active)
            .find(|c| of_provider.iter().any(|a| a.id == c.id && a.peak().is_some_and(|p| p < threshold - HYSTERESIS)));
        if let Some(target) = best {
            out.push((provider, target.id.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RemoteSeat, UsageWindow};

    fn account(id: &str, active: bool, percent: Option<f64>) -> AccountUsage {
        AccountUsage {
            id: id.into(), provider: Provider::Claude, email: None, alias: None, plan: None, org: None,
            active, sources: vec![], windows: percent.map(|used_percent| UsageWindow {
                kind: "session".into(), window_seconds: None, used_percent, resets_at: None,
            }).into_iter().collect(), credits: None, resets: vec![], error: None, monitor_error: None, stale: false,
            fetched_at: Some(chrono::Utc::now()), monitor_connected: true, billing: Default::default(), remote: vec![], used_outside: false, runs_out_at: None, unexpected: None, cli_rejected: false,
        }
    }

    #[test]
    fn automatic_switching_requires_fresh_measured_usage_on_both_accounts() {
        let mut settings = Settings::default();
        settings.balancer.claude.auto = true;
        let current = account("active", true, Some(95.0));
        let mut target = account("inactive", false, Some(10.0));
        target.stale = true;
        assert!(decide(&[current.clone(), target.clone()], &settings, &HashMap::new()).is_empty());
        target.stale = false;
        target.error = Some("monitor_network".into());
        assert!(decide(&[current.clone(), target.clone()], &settings, &HashMap::new()).is_empty());
        assert!(decide(&[current.clone(), account("unknown", false, None)], &settings, &HashMap::new()).is_empty());
        target.error = None;
        // A monitor error must not invalidate a successful live CLI measurement.
        target.monitor_error = Some("monitor_reconnect".into());
        assert_eq!(decide(&[current.clone(), target.clone()], &settings, &HashMap::new()), [(Provider::Claude, "inactive".into())]);
        let mut stale_current = current;
        stale_current.stale = true;
        assert!(decide(&[stale_current, target], &settings, &HashMap::new()).is_empty());
    }

    #[test]
    fn a_lapsing_lapsed_or_ending_plan_is_never_a_switch_target() {
        let mut settings = Settings::default();
        settings.balancer.claude.auto = true;
        let accounts = [account("active", true, Some(95.0)), account("inactive", false, Some(10.0))];
        assert_eq!(decide(&accounts, &settings, &HashMap::new()).len(), 1);
        let plan = |days: i64| crate::settings::Subscription { cancel_after: Some((chrono::Utc::now() + chrono::Duration::days(days)).date_naive()), ..Default::default() };
        settings.subscriptions.insert("claude:inactive".into(), plan(3));
        assert!(decide(&accounts, &settings, &HashMap::new()).is_empty(), "being let go");
        settings.subscriptions.insert("claude:inactive".into(), plan(-1));
        assert!(decide(&accounts, &settings, &HashMap::new()).is_empty());
        settings.subscriptions.clear();
        let mut ending = accounts.clone();
        ending[1].billing.will_renew = Some(false);
        ending[1].billing.active_until = Some(chrono::Utc::now() + chrono::Duration::days(3));
        assert!(decide(&ending, &settings, &HashMap::new()).is_empty(), "cancelled with the provider");
    }

    fn weekly(id: &str, used: f64, resets_in_hours: i64) -> AccountUsage {
        let mut a = account(id, false, Some(5.0));
        a.windows.push(UsageWindow { kind: "weekly".into(), window_seconds: None, used_percent: used,
            resets_at: Some(chrono::Utc::now() + chrono::Duration::hours(resets_in_hours)) });
        a
    }

    #[test]
    fn prefers_the_target_whose_capacity_is_lost_soonest_and_never_one_in_use_elsewhere() {
        let mut settings = Settings::default();
        settings.balancer.claude.auto = true;
        let current = account("active", true, Some(95.0));
        // "emptiest" has the most room, but "soon" loses its unused week first.
        let soon = weekly("soon", 60.0, 10);
        let emptiest = weekly("emptiest", 0.0, 150);
        let later = weekly("later", 20.0, 40);
        let all = [current.clone(), emptiest.clone(), later.clone(), soon.clone()];
        assert_eq!(decide(&all, &settings, &HashMap::new()), [(Provider::Claude, "soon".into())]);
        // Too close to the threshold to leave room (hysteresis): the next soonest instead.
        let mut full = soon.clone();
        full.windows[1].used_percent = 85.0;
        assert_eq!(decide(&[current.clone(), emptiest.clone(), later.clone(), full], &settings, &HashMap::new()), [(Provider::Claude, "later".into())]);
        // Live on another machine: never a target, however early it resets.
        let mut remote = soon.clone();
        remote.remote = vec![RemoteSeat { machine: "DESKTOP-TEST".into(), where_: vec![], seen_at: chrono::Utc::now(), live: true }];
        assert_eq!(decide(&[current.clone(), emptiest.clone(), remote], &settings, &HashMap::new()), [(Provider::Claude, "emptiest".into())]);
        // A renewal before its weekly reset counts as losing capacity too.
        let mut renews = weekly("renews", 30.0, 150);
        renews.billing.renews_at = Some(chrono::Utc::now() + chrono::Duration::hours(5));
        renews.plan = Some("max".into());
        assert_eq!(decide(&[current.clone(), later.clone(), renews], &settings, &HashMap::new()), [(Provider::Claude, "renews".into())]);
        // The cooldown still holds.
        let last = HashMap::from([(Provider::Claude, Instant::now())]);
        assert!(decide(&all, &settings, &last).is_empty());
    }
}
