//! Codex reset credits: when an account hits its limit (or will within a few hours) while it holds
//! a reset credit, whether to spend the credit or switch to another account with room. A credit
//! about to expire is worth spending; one that keeps is worth saving while another account can
//! take the work; a limit that comes back within the hour or two needs neither. Separately, a
//! credit still held two days before it expires is worth a reminder. SlopBar never spends one:
//! the user does, in Codex.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::accounts::Provider;
use crate::model::{AccountUsage, ResetGrant};
use crate::plan_alerts::{Sent, RUNS_OUT_LEAD_HOURS};

/// A credit expiring within this many hours is spent rather than saved, and reminded about.
pub const EXPIRY_REMINDER_H: i64 = 48;
/// Another account needs this much left of every window to take the work.
const MIN_SWITCH_LEFT: f64 = 20.0;
/// A limit that resets within this many hours is worth waiting for.
const WAIT_H: i64 = 2;
const LIMIT_PERCENT: f64 = 99.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice { Spend, Switch, Wait }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The credit expires within two days, or before the limit would reset anyway: use it or lose it.
    ExpiresSoon,
    /// Another account has room: switch and keep the credit for later.
    OtherHasHeadroom,
    /// The limit comes back within a couple of hours.
    ResetsSoon,
    /// No other account has room, and the limit is hours away.
    NoOtherAccount,
}

#[derive(Debug, Clone, Serialize)]
pub struct Alternative {
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    /// The least left of its windows, in percent.
    pub left_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreditAdvice {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    /// A window is at its limit now; otherwise the week runs out at `limit_at`.
    pub at_limit: bool,
    pub limit_at: Option<DateTime<Utc>>,
    /// When the limiting window comes back on its own.
    pub resets_at: Option<DateTime<Utc>>,
    /// Reset credits held, whether one can be spent right now, and when the first expires.
    pub credits: u32,
    pub usable_now: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub choice: Choice,
    pub reason: Reason,
    pub switch_to: Option<Alternative>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Expiring {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    /// The credit's kind ("full", "session", "weekly").
    pub reset_kind: String,
    pub count: u32,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreditAlert {
    Advice(CreditAdvice),
    Expiring(Expiring),
}

fn held(acc: &AccountUsage, now: DateTime<Utc>) -> impl Iterator<Item = &ResetGrant> {
    acc.resets.iter().filter(move |r| r.count > 0 && r.expires_at.is_none_or(|e| e > now))
}

fn readable(acc: &AccountUsage) -> bool {
    acc.provider == Provider::Codex && !acc.stale && acc.error.is_none()
}

/// The least left across an account's windows, in percent.
fn left(acc: &AccountUsage) -> f64 {
    100.0 - acc.peak().unwrap_or(0.0)
}

/// Advice for every Codex account at or near its limit that holds a reset credit.
pub fn advise(accounts: &[AccountUsage], now: DateTime<Utc>) -> Vec<CreditAdvice> {
    let alternative = |not: &str| accounts.iter()
        .filter(|a| readable(a) && a.id != not && a.runs_out_at.is_none() && left(a) >= MIN_SWITCH_LEFT)
        .max_by(|a, b| left(a).total_cmp(&left(b)).then_with(|| b.id.cmp(&a.id)))
        .map(|a| Alternative { id: a.id.clone(), email: a.email.clone(), alias: a.alias.clone(), left_percent: left(a) });
    accounts.iter().filter(|a| readable(a)).filter_map(|acc| {
        let credits: Vec<&ResetGrant> = held(acc, now).collect();
        if credits.is_empty() { return None; }
        let limited: Vec<_> = acc.windows.iter().filter(|w| w.used_percent >= LIMIT_PERCENT).collect();
        let at_limit = !limited.is_empty();
        let soon = acc.runs_out_at.filter(|t| *t - now <= Duration::hours(RUNS_OUT_LEAD_HOURS));
        if !at_limit && soon.is_none() { return None; }
        // Usable again when the last limited window resets.
        let resets_at = if at_limit { limited.iter().filter_map(|w| w.resets_at).max() }
            else { acc.windows.iter().find(|w| w.kind == "weekly").and_then(|w| w.resets_at) };
        let expires_at = credits.iter().filter_map(|r| r.expires_at).min();
        let switch_to = alternative(&acc.id);
        let expiring = expires_at.is_some_and(|e| e - now <= Duration::hours(EXPIRY_REMINDER_H) || resets_at.is_some_and(|r| e < r));
        let (choice, reason) = if expiring { (Choice::Spend, Reason::ExpiresSoon) }
            else if switch_to.is_some() { (Choice::Switch, Reason::OtherHasHeadroom) }
            else if resets_at.is_some_and(|r| r - now <= Duration::hours(WAIT_H)) { (Choice::Wait, Reason::ResetsSoon) }
            else { (Choice::Spend, Reason::NoOtherAccount) };
        Some(CreditAdvice {
            provider: acc.provider, id: acc.id.clone(), email: acc.email.clone(), alias: acc.alias.clone(),
            at_limit, limit_at: if at_limit { None } else { soon }, resets_at,
            credits: credits.iter().map(|r| r.count).sum(), usable_now: credits.iter().any(|r| r.usable), expires_at,
            choice, reason, switch_to: (choice == Choice::Switch).then_some(switch_to).flatten(),
        })
    }).collect()
}

/// Reset credits still held within two days of expiring.
pub fn expiring(accounts: &[AccountUsage], now: DateTime<Utc>) -> Vec<Expiring> {
    accounts.iter().filter(|a| a.provider == Provider::Codex).flat_map(|acc| held(acc, now)
        .filter(|r| r.expires_at.is_some_and(|e| e - now <= Duration::hours(EXPIRY_REMINDER_H)))
        .map(|r| Expiring { provider: acc.provider, id: acc.id.clone(), email: acc.email.clone(), alias: acc.alias.clone(),
            reset_kind: r.kind.clone(), count: r.count, expires_at: r.expires_at.unwrap() })).collect()
}

/// The reset-credit notifications due now that were not shown before, marked in the plan
/// notifications' store: advice once per account and limited window, a reminder once per credit.
pub fn due(accounts: &[AccountUsage], enabled: bool, sent: &mut Sent, now: DateTime<Utc>) -> Vec<CreditAlert> {
    if !enabled { return vec![]; }
    let mut mark = |key: String| sent.sent.insert(key, now).is_none();
    let mut out = vec![];
    for a in advise(accounts, now) {
        let window = a.resets_at.map_or(0, |r| r.timestamp() / 3600);
        if mark(format!("reset_credit:{}:{}:{window}", a.provider.name(), a.id)) { out.push(CreditAlert::Advice(a)); }
    }
    for e in expiring(accounts, now) {
        if mark(format!("reset_credit_expiring:{}:{}:{}:{}", e.provider.name(), e.id, e.reset_kind, e.expires_at.timestamp())) { out.push(CreditAlert::Expiring(e)); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_alerts::tests::{account, at};

    const NOW: &str = "2026-10-21T12:00:00Z";

    fn grant(usable: bool, expires_in_h: Option<i64>) -> ResetGrant {
        ResetGrant { kind: "full".into(), title: None, count: 1, usable, expires_at: expires_in_h.map(|h| at(NOW) + Duration::hours(h)), next_available_at: None }
    }

    fn codex(id: &str, used: f64, resets_in_h: i64, grants: Vec<ResetGrant>) -> AccountUsage {
        let mut a = account(id, used, at(NOW) + Duration::hours(resets_in_h));
        a.resets = grants;
        a
    }

    #[test]
    fn switch_while_another_account_has_room_and_spend_what_would_expire() {
        let now = at(NOW);
        let spare = codex("spare", 30.0, 72, vec![]);
        let tight = codex("tight", 90.0, 72, vec![]);
        // At its limit for three more days, holding a credit good for a week; another account has 70% left.
        let full = codex("full", 100.0, 72, vec![grant(true, Some(24 * 7))]);
        let a = &advise(&[full.clone(), spare.clone(), tight.clone()], now)[0];
        assert_eq!((a.choice, a.reason, a.at_limit, a.credits, a.usable_now), (Choice::Switch, Reason::OtherHasHeadroom, true, 1, true));
        assert_eq!(a.switch_to.as_ref().map(|s| (s.id.as_str(), s.left_percent)), Some(("spare", 70.0)));
        assert_eq!(a.resets_at, Some(now + Duration::hours(72)));

        // The same credit expiring tomorrow is spent, even with room elsewhere.
        let expiring = codex("full", 100.0, 72, vec![grant(true, Some(30))]);
        let a = &advise(&[expiring, spare.clone()], now)[0];
        assert_eq!((a.choice, a.reason, a.switch_to.is_none()), (Choice::Spend, Reason::ExpiresSoon, true));
        // Expiring before the limit would come back counts the same, whatever the two days.
        let a = &advise(&[codex("full", 100.0, 24 * 6, vec![grant(true, Some(24 * 5))]), spare.clone()], now)[0];
        assert_eq!(a.reason, Reason::ExpiresSoon);

        // No room anywhere: wait for a limit that is back within two hours, else spend.
        let a = &advise(&[codex("full", 100.0, 1, vec![grant(true, None)]), tight.clone()], now)[0];
        assert_eq!((a.choice, a.reason), (Choice::Wait, Reason::ResetsSoon));
        let a = &advise(&[codex("full", 100.0, 30, vec![grant(true, None)]), tight.clone()], now)[0];
        assert_eq!((a.choice, a.reason), (Choice::Spend, Reason::NoOtherAccount));
        // An alternative that runs out before its reset is no room either.
        let mut busy = spare.clone();
        busy.runs_out_at = Some(now + Duration::hours(5));
        assert_eq!(advise(&[codex("full", 100.0, 30, vec![grant(true, None)]), busy], now)[0].reason, Reason::NoOtherAccount);
    }

    #[test]
    fn only_accounts_near_their_limit_with_a_credit_get_advice() {
        let now = at(NOW);
        // About to run out within three hours: advised before the credit is usable.
        let mut near = codex("near", 95.0, 72, vec![grant(false, None)]);
        near.runs_out_at = Some(now + Duration::hours(2));
        let a = &advise(&[near.clone(), codex("spare", 10.0, 72, vec![])], now)[0];
        assert_eq!((a.at_limit, a.usable_now, a.limit_at, a.choice), (false, false, Some(now + Duration::hours(2)), Choice::Switch));
        near.runs_out_at = Some(now + Duration::hours(5));
        assert!(advise(&[near], now).is_empty(), "runs out later than three hours");
        assert!(advise(&[codex("x", 100.0, 72, vec![])], now).is_empty(), "no credit");
        assert!(advise(&[codex("x", 100.0, 72, vec![grant(true, Some(-1))])], now).is_empty(), "expired credit");
        let mut stale = codex("x", 100.0, 72, vec![grant(true, None)]);
        stale.stale = true;
        assert!(advise(&[stale], now).is_empty());
        let mut claude = codex("x", 100.0, 72, vec![grant(true, None)]);
        claude.provider = Provider::Claude;
        assert!(advise(&[claude], now).is_empty(), "Codex only");
    }

    #[test]
    fn notifications_come_once_per_window_and_credit_and_respect_the_toggle() {
        let now = at(NOW);
        let list = vec![codex("full", 100.0, 72, vec![grant(true, Some(40))]), codex("idle", 5.0, 72, vec![grant(false, Some(47)), grant(false, Some(60))])];
        let mut sent = Sent::default();
        assert!(due(&list, false, &mut sent, now).is_empty() && sent.sent.is_empty());
        let first = due(&list, true, &mut sent, now);
        let kinds: Vec<String> = first.iter().map(|a| serde_json::to_value(a).unwrap()["kind"].as_str().unwrap().to_string()).collect();
        // Advice for the full account; reminders for its credit and the idle account's sooner one
        // (the other expires in 60 hours).
        assert_eq!(kinds, ["advice", "expiring", "expiring"]);
        let CreditAlert::Expiring(e) = &first[2] else { panic!() };
        assert_eq!((e.id.as_str(), e.expires_at), ("idle", now + Duration::hours(47)));
        assert!(due(&list, true, &mut sent, now + Duration::hours(1)).is_empty(), "each once");
        assert_eq!(serde_json::to_value(&first[0]).unwrap()["choice"], "spend");
    }
}
