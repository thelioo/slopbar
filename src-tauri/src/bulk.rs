//! Bulk account actions: which accounts "Connect monitoring for all" and "Remove lapsed accounts"
//! act on. The app carries them out with the same one-at-a-time operations as each row's buttons.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

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

/// An account a restored backup knows that is not signed in here yet. The backup names it, and
/// that name exists only in the backup file, so it is kept here (`pending-signins.json`, the
/// app's data folder) until the account is signed in again. Never any sign-in data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
}

fn pending_path() -> Option<std::path::PathBuf> {
    Some(crate::settings::app_dir()?.join("pending-signins.json"))
}

pub fn load_pending() -> Vec<Pending> {
    pending_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_pending(list: &[Pending]) -> std::io::Result<()> {
    let path = pending_path().ok_or_else(|| std::io::Error::other("no data folder"))?;
    if list.is_empty() {
        return match std::fs::remove_file(&path) { Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e), _ => Ok(()) };
    }
    crate::private_store::write(&path, &serde_json::to_vec_pretty(list)?)
}

/// Adds a restore's accounts that are not signed in here to `list`, once each.
pub fn add_pending(list: &mut Vec<Pending>, restored: impl IntoIterator<Item = Pending>) {
    for p in restored {
        match list.iter_mut().find(|q| q.provider == p.provider && q.id == p.id) {
            Some(q) => { if p.email.is_some() { q.email = p.email; } }
            None => list.push(p),
        }
    }
}

/// One sign-in of "Sign in all accounts": `Add` saves a pending account's login in the vault (the
/// account appears), `Connect` gives an account its monitoring sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Add,
    Connect,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub kind: Kind,
    #[serde(flatten)]
    pub target: Target,
    /// Who must sign in: the account's email, prefilled where the CLI can.
    pub email: Option<String>,
}

/// The sign-ins for one email, run back to back in that email's browser profile, so Google is
/// asked once for all of them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    pub email: Option<String>,
    pub steps: Vec<Step>,
}

/// What "Sign in all accounts" does: every pending account signed in again, and, with
/// `monitoring`, a monitoring sign-in for each account without one, grouped by email in the order
/// the emails first appear. Within an email: Claude before Codex, each account's own sign-in before
/// its monitoring one. An account without an email is a group of its own.
pub fn sign_in_plan(snapshot: &Snapshot, pending: &[Pending], monitoring: bool) -> Vec<Group> {
    let mut steps: Vec<Step> = Vec::new();
    for p in pending {
        if snapshot.accounts.iter().any(|a| a.provider == p.provider && a.id == p.id) {
            continue;
        }
        let target = Target { provider: p.provider, id: p.id.clone(), account: p.email.clone(), ended: None };
        steps.push(Step { kind: Kind::Add, target: target.clone(), email: p.email.clone() });
        if monitoring {
            steps.push(Step { kind: Kind::Connect, target, email: p.email.clone() });
        }
    }
    if monitoring {
        for a in snapshot.accounts.iter().filter(|a| !a.monitor_connected) {
            steps.push(Step { kind: Kind::Connect, target: target(a, None), email: a.email.clone() });
        }
    }
    let key = |s: &Step| s.email.as_deref().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty())
        .unwrap_or_else(|| format!("\0{}:{}", s.target.provider.key(), s.target.id));
    let mut groups: Vec<(String, Group)> = Vec::new();
    for step in steps {
        let k = key(&step);
        match groups.iter_mut().find(|(g, _)| *g == k) {
            Some((_, g)) => g.steps.push(step),
            None => groups.push((k, Group { email: step.email.clone(), steps: vec![step] })),
        }
    }
    for (_, g) in &mut groups {
        // Stable: an account's Add stays before its Connect, accounts keep their order.
        g.steps.sort_by_key(|s| s.target.provider != Provider::Claude);
    }
    groups.into_iter().map(|(_, g)| g).collect()
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

    fn pending(provider: Provider, id: &str, email: Option<&str>) -> Pending {
        Pending { provider, id: id.into(), email: email.map(Into::into) }
    }

    fn summary(groups: &[Group]) -> Vec<(Option<&str>, Vec<String>)> {
        groups.iter().map(|g| (g.email.as_deref(), g.steps.iter().map(|s| format!("{:?} {} {}",
            s.kind, s.target.provider.key(), s.target.id)).collect())).collect()
    }

    #[test]
    fn a_restore_signs_in_again_one_email_at_a_time_claude_first() {
        // A new machine: four restored accounts across two emails, Codex listed first.
        let list = [
            pending(Provider::Codex, "cx-one", Some("one@example.invalid")),
            pending(Provider::Claude, "cl-two", Some("two@example.invalid")),
            pending(Provider::Claude, "cl-one", Some("One@Example.invalid")),
            pending(Provider::Codex, "cx-two", Some("two@example.invalid")),
        ];
        let empty = Snapshot { accounts: vec![], updated_at: None };
        let plan = sign_in_plan(&empty, &list, false);
        assert_eq!(summary(&plan), [
            (Some("one@example.invalid"), vec!["Add claude cl-one".to_string(), "Add codex cx-one".into()]),
            (Some("two@example.invalid"), vec!["Add claude cl-two".to_string(), "Add codex cx-two".into()]),
        ], "two emails, two Google sign-ins; the email's case does not split a group");
        let with_monitoring = sign_in_plan(&empty, &list, true);
        assert_eq!(summary(&with_monitoring)[0].1, ["Add claude cl-one", "Connect claude cl-one", "Add codex cx-one", "Connect codex cx-one"]);
        assert_eq!(with_monitoring[0].steps[0].email.as_deref(), Some("One@Example.invalid"), "each step keeps the account's own email to prefill");
    }

    #[test]
    fn signed_in_accounts_are_not_pending_and_only_unmonitored_ones_connect() {
        let mut here = account("cl-one", false, false);
        here.email = Some("one@example.invalid".into());
        let mut monitored = account("cl-three", false, true);
        monitored.email = Some("one@example.invalid".into());
        let mut nameless = account("cl-four", false, false);
        nameless.email = None;
        let snapshot = Snapshot { accounts: vec![here, monitored, nameless], updated_at: None };
        let list = [pending(Provider::Claude, "cl-one", Some("one@example.invalid")), pending(Provider::Codex, "cx-one", Some("one@example.invalid"))];
        assert_eq!(summary(&sign_in_plan(&snapshot, &list, true)), [
            (Some("one@example.invalid"), vec!["Connect claude cl-one".to_string(), "Add codex cx-one".into(), "Connect codex cx-one".into()]),
            (None, vec!["Connect claude cl-four".to_string()]),
        ]);
        assert!(sign_in_plan(&snapshot, &list[..1], false).is_empty(), "nothing pending, nothing to sign in");
    }

    #[test]
    fn restores_add_pending_accounts_once() {
        let mut list = vec![pending(Provider::Claude, "a", None)];
        add_pending(&mut list, [pending(Provider::Claude, "a", Some("a@example.invalid")), pending(Provider::Codex, "a", None)]);
        add_pending(&mut list, [pending(Provider::Claude, "a", None)]);
        assert_eq!(list, [pending(Provider::Claude, "a", Some("a@example.invalid")), pending(Provider::Codex, "a", None)],
            "a later restore without the email keeps the one known; the same id of another provider is another account");
    }
}
