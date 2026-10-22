//! Bulk account actions: which accounts "Connect monitoring for all" and "Remove lapsed accounts"
//! act on. The app carries them out with the same one-at-a-time operations as each row's buttons.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use crate::accounts::Provider;
use crate::model::{AccountUsage, Snapshot};
use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Target {
    pub provider: Provider,
    pub id: String,
    /// Alias, else email: what the user signs in as, or sees removed.
    pub account: Option<String>,
    /// Lapsed accounts: the day the plan ended (the last paid day the user entered, or the end
    /// the provider gave).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<NaiveDate>,
}

fn target(a: &AccountUsage, ended: Option<NaiveDate>) -> Target {
    Target { provider: a.provider, id: a.id.clone(), account: a.alias.clone().or(a.email.clone()), ended }
}

/// Accounts without independent monitoring, in the order Settings lists them.
pub fn unmonitored(snapshot: &Snapshot) -> Vec<Target> {
    snapshot.accounts.iter().filter(|a| !a.monitor_connected).map(|a| target(a, None)).collect()
}

/// The day an account's plan ended, if it has: the user's last paid day is past, or the provider
/// said the plan would not renew and its end has passed.
pub fn ended(a: &AccountUsage, settings: &Settings, now: DateTime<Utc>) -> Option<NaiveDate> {
    let entered = settings.subscriptions.get(&format!("{}:{}", a.provider.key(), a.id)).and_then(|p| p.cancel_after)
        .filter(|d| *d < now.date_naive());
    let b = &a.billing;
    let ending = b.cancels_at.is_some() || b.will_renew == Some(false);
    let provider = ending.then(|| b.cancels_at.or(b.active_until).or(b.renews_at)).flatten()
        .filter(|t| *t <= now).map(|t| t.date_naive());
    entered.or(provider)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Lapsed {
    /// Can be removed now.
    pub remove: Vec<Target>,
    /// The CLI is signed in to them: switch away first.
    pub in_use: Vec<Target>,
}

pub fn lapsed(snapshot: &Snapshot, settings: &Settings, now: DateTime<Utc>) -> Lapsed {
    let mut out = Lapsed::default();
    for a in &snapshot.accounts {
        if let Some(day) = ended(a, settings, now) {
            if a.active { out.in_use.push(target(a, Some(day))) } else { out.remove.push(target(a, Some(day))) }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Subscription;

    fn account(id: &str, active: bool, monitored: bool) -> AccountUsage {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "claude", "email": format!("{id}@example.invalid"), "alias": null, "plan": "max", "org": null,
            "active": active, "sources": [], "windows": [], "credits": null, "resets": [], "error": null,
            "stale": false, "fetched_at": null, "monitor_connected": monitored,
        })).unwrap()
    }

    #[test]
    fn unmonitored_lists_only_accounts_without_monitoring() {
        let mut named = account("b", false, false);
        named.alias = Some("Spare".into());
        let snapshot = Snapshot { accounts: vec![account("a", true, true), named, account("c", true, false)], updated_at: None };
        let found = unmonitored(&snapshot);
        assert_eq!(found.iter().map(|t| (t.id.as_str(), t.account.as_deref())).collect::<Vec<_>>(),
            [("b", Some("Spare")), ("c", Some("c@example.invalid"))]);
    }

    #[test]
    fn lapsed_means_past_the_last_paid_day_or_a_provider_end_that_passed() {
        let now: DateTime<Utc> = "2026-10-21T12:00:00Z".parse().unwrap();
        let mut settings = Settings::default();
        let day = |s: &str| s.parse::<NaiveDate>().unwrap();
        settings.subscriptions.insert("claude:entered".into(), Subscription { cancel_after: Some(day("2026-10-20")), ..Default::default() });
        settings.subscriptions.insert("claude:today".into(), Subscription { cancel_after: Some(day("2026-10-21")), ..Default::default() });
        settings.subscriptions.insert("claude:busy".into(), Subscription { cancel_after: Some(day("2026-10-01")), ..Default::default() });
        let mut cancelled = account("cancelled", false, true);
        cancelled.billing.cancels_at = Some("2026-10-18T00:00:00Z".parse().unwrap());
        let mut ending = account("ending", false, true);
        ending.billing.will_renew = Some(false);
        ending.billing.active_until = Some("2026-10-30T00:00:00Z".parse().unwrap());
        let mut renewing = account("renewing", false, true);
        renewing.billing.active_until = Some("2026-10-01T00:00:00Z".parse().unwrap());
        let snapshot = Snapshot { accounts: vec![
            account("entered", false, false), account("today", false, false), account("busy", true, true), cancelled, ending, renewing,
        ], updated_at: None };
        let found = lapsed(&snapshot, &settings, now);
        assert_eq!(found.remove.iter().map(|t| (t.id.as_str(), t.ended)).collect::<Vec<_>>(),
            [("entered", Some(day("2026-10-20"))), ("cancelled", Some(day("2026-10-18")))],
            "the last paid day itself still counts, an end still ahead and a period end without cancelling do not");
        assert_eq!(found.in_use.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["busy"], "the account in use is never removed");
    }
}
