//! Automatic account switching: when the signed-in account of a provider reaches the
//! configured share of any usage window, move to the account with the most headroom.

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
        // Switch only to measured, current headroom.
        let best = of_provider
            .iter()
            .filter(|a| !a.active)
            .filter(|a| a.peak().is_some_and(|p| p < threshold - HYSTERESIS))
            .min_by(|a, b| {
                let key = |x: &AccountUsage| x.peak().unwrap_or(f64::MAX);
                key(a).total_cmp(&key(b))
            });
        if let Some(target) = best {
            out.push((provider, target.id.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageWindow;

    fn account(id: &str, active: bool, percent: Option<f64>) -> AccountUsage {
        AccountUsage {
            id: id.into(), provider: Provider::Claude, email: None, alias: None, plan: None, org: None,
            active, sources: vec![], windows: percent.map(|used_percent| UsageWindow {
                kind: "session".into(), window_seconds: None, used_percent, resets_at: None,
            }).into_iter().collect(), credits: None, resets: vec![], error: None, monitor_error: None, stale: false,
            fetched_at: Some(chrono::Utc::now()), monitor_connected: true,
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
}
