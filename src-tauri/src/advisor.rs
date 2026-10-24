//! Plan change advice: concrete moves per provider from the report's verdicts, pool sizing and
//! tiers. Cancel plans that look unused, move underused ones to a cheaper tier that still fits
//! their busiest recent week, and never leave a provider with less than its pool's busiest weeks
//! (90th percentile) plus 20%. Each move says what it saves a month; the provider says how much
//! capacity would be left once every move is made.
//!
//! The other way round, a provider whose busiest weeks plus 20% need more than its plans hold, or
//! whose plans keep running out with little to spare, gets the cheapest fix with its monthly cost:
//! upgrade one plan, or add one of the tier that fits. Never in the same advice as a cut.
//!
//! With too little history to size the pool, only clear evidence counts: plans judged unused
//! (weeks watched near zero, or a Codex plan idle for weeks on this PC's history). SlopBar cannot
//! change a plan; every move links to the provider's own billing page.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::accounts::Provider;
use crate::subscriptions::{billing_page, reference_plan, Basis, Entry, PlanKind, ProviderSummary, Verdict, POOL_BUFFER};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Cancel,
    Downgrade,
}

/// What the advice rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// The pool has enough weeks: every move keeps the busiest weeks plus 20% covered.
    Pool,
    /// Not enough history to size the pool: only plans judged unused, and no downgrades.
    ClearOnly,
}

#[derive(Debug, Clone, Serialize)]
pub struct Move {
    pub provider: Provider,
    pub id: String,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub action: Action,
    /// The plan now, and the tier to move to (downgrades only).
    pub from: String,
    pub to: Option<String>,
    /// What the move saves a month, in dollars.
    pub monthly_saving: f64,
    pub verdict: Verdict,
    pub basis: Basis,
    /// The busiest recent week, in percent of this plan's allowance.
    pub peak_percent: Option<f64>,
    /// The next renewal: the move is worth making before it.
    pub before: Option<DateTime<Utc>>,
    /// Where the plan is changed (fixed per provider).
    pub manage_url: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAdvice {
    pub provider: Provider,
    pub evidence: Evidence,
    /// The plan capacity is counted in (`subscriptions::reference_plan`).
    pub unit: &'static str,
    /// Weeks of history the pool rests on, and how many it needs.
    pub weeks_used: u8,
    pub weeks_needed: u8,
    /// Paid plans that keep renewing, in `unit`, before and after the moves.
    pub capacity_now: f64,
    pub capacity_after: f64,
    /// The pool's 90th-percentile week, and the least capacity any advice may leave (that + 20%;
    /// this week's use + 20% when the pool has too little history).
    pub demand_p90: Option<f64>,
    pub floor: Option<f64>,
    pub moves: Vec<Move>,
    pub monthly_saving: f64,
    /// Capacity left over the busiest weeks after every move, in `unit` and in percent.
    pub spare_after: Option<f64>,
    pub spare_after_percent: Option<f64>,
    /// Unused or underused plans kept because moving them would cut below the floor.
    pub kept_for_capacity: usize,
    /// Too little capacity, and the cheapest fix. Only when there are no moves: a provider is
    /// never told to cancel or downgrade and to add in the same advice.
    pub shortage: Option<Shortage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortageCause {
    /// The busiest weeks plus 20% need more than the renewing plans hold.
    OverCapacity,
    /// Plans keep reaching their weekly limit with under 40% to spare over the busiest weeks:
    /// use that hits a limit is cut short, so the real demand is higher than measured.
    HittingLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixAction {
    /// Move one plan to a bigger tier.
    Upgrade,
    /// Buy another plan.
    Add,
}

#[derive(Debug, Clone, Serialize)]
pub struct Shortage {
    pub cause: ShortageCause,
    /// Capacity missing, in `unit`.
    pub shortfall: f64,
    /// Plans judged to run out most weeks.
    pub exhausted: usize,
    pub fix: Fix,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    pub action: FixAction,
    /// Upgrade: the account and its plan now.
    pub id: Option<String>,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub from: Option<String>,
    pub to: String,
    /// Plans of `to` to add: one, unless no single tier covers the shortfall.
    pub count: u32,
    /// What the fix adds to the monthly bill, in dollars (list prices).
    pub monthly_cost: f64,
    pub capacity_after: f64,
    pub manage_url: &'static str,
}

/// Tiers a provider sells, largest first: (name, weekly allowance in `reference_plan`s, US list
/// price per month). Same weights as `subscriptions::plan_weight`.
fn tiers(provider: Provider) -> &'static [(&'static str, f64, f64)] {
    match provider {
        Provider::Claude => &[("Max 20x", 1.0, 200.0), ("Max 5x", 0.25, 100.0), ("Pro", 0.05, 20.0)],
        Provider::Codex => &[("Pro 500", 2.5, 500.0), ("Pro 200", 1.0, 200.0), ("Pro 100", 0.5, 100.0), ("Plus", 0.1, 20.0)],
    }
}

/// A plan the advisor may move: paid by the user, still renewing, of known size and cost.
struct Candidate<'a> {
    entry: &'a Entry,
    plan: String,
    weight: f64,
    cost: f64,
}

pub fn advise(accounts: &[Entry], providers: &[ProviderSummary]) -> Vec<ProviderAdvice> {
    providers.iter().filter(|s| s.paid > 0).map(|s| advise_provider(s, accounts)).collect()
}

fn advise_provider(summary: &ProviderSummary, accounts: &[Entry]) -> ProviderAdvice {
    let provider = summary.provider;
    let pool = &summary.pool;
    // Plans already ending or being let lapse are capacity that is going away anyway.
    let renewing: Vec<&Entry> = accounts.iter()
        .filter(|e| e.provider == provider && e.plan_kind == PlanKind::Personal && e.ends_at.is_none() && e.cancel_after.is_none())
        .collect();
    let capacity_now: f64 = renewing.iter().filter_map(|e| e.plan_weight).sum();
    let evidence = if pool.demand_p90.is_some() { Evidence::Pool } else { Evidence::ClearOnly };
    let floor = match evidence {
        Evidence::Pool => pool.demand_p90.map(|d| d * (1.0 + POOL_BUFFER)),
        Evidence::ClearOnly => pool.live_demand.map(|d| d * (1.0 + POOL_BUFFER)),
    };
    let mut candidates: Vec<Candidate> = renewing.iter().filter_map(|e| Some(Candidate {
        entry: e, plan: e.plan.clone()?, weight: e.plan_weight?, cost: e.monthly_cost?,
    })).collect();
    // The biggest savings first, then a stable order.
    candidates.sort_by(|a, b| b.cost.total_cmp(&a.cost).then_with(|| a.entry.id.cmp(&b.entry.id)));

    let mut capacity = capacity_now;
    let mut plans_left = renewing.len();
    let mut moves = Vec::new();
    let mut kept = 0;
    let fits = |after: f64| floor.is_none_or(|f| after + 1e-9 >= f);
    let movable = |c: &Candidate| match evidence {
        Evidence::Pool => matches!(c.entry.assessment.verdict, Verdict::Unused | Verdict::Underused),
        Evidence::ClearOnly => c.entry.assessment.verdict == Verdict::Unused,
    };
    // Cancels first: an unused plan frees its whole price.
    for c in candidates.iter().filter(|c| movable(c) && c.entry.assessment.verdict == Verdict::Unused) {
        // Never advise dropping a provider altogether.
        if plans_left <= 1 || !fits(capacity - c.weight) { kept += 1; continue; }
        capacity -= c.weight;
        plans_left -= 1;
        moves.push(new_move(c, Action::Cancel, None, c.cost));
    }
    // Then underused plans move to the cheapest smaller tier that still fits their busiest week.
    if evidence == Evidence::Pool {
        for c in candidates.iter().filter(|c| c.entry.assessment.verdict == Verdict::Underused) {
            let Some(peak) = c.entry.recent_peak() else { kept += 1; continue };
            let need = peak / 100.0 * c.weight * (1.0 + POOL_BUFFER);
            let target = tiers(provider).iter().rev()
                .filter(|(_, w, price)| *w < c.weight && *price < c.cost)
                .find(|(_, w, _)| *w + 1e-9 >= need && fits(capacity - (c.weight - w)));
            match target {
                Some((name, w, price)) => {
                    capacity -= c.weight - w;
                    moves.push(new_move(c, Action::Downgrade, Some(name.to_string()), c.cost - price));
                }
                None => kept += 1,
            }
        }
    }
    let demand = pool.demand_p90.or(match evidence { Evidence::ClearOnly => pool.live_demand, Evidence::Pool => None });
    let shortage = if moves.is_empty() { shortage(provider, &candidates, capacity_now, demand, summary.exhausted) } else { None };
    ProviderAdvice {
        provider,
        evidence,
        unit: reference_plan(provider),
        weeks_used: pool.weeks_used,
        weeks_needed: pool.weeks_needed,
        capacity_now: round(capacity_now),
        capacity_after: round(capacity),
        demand_p90: pool.demand_p90,
        floor: floor.map(round),
        monthly_saving: round(moves.iter().map(|m: &Move| m.monthly_saving).sum()),
        spare_after: demand.map(|d| round(capacity - d)),
        spare_after_percent: demand.filter(|d| *d > 0.0).map(|d| round((capacity / d - 1.0) * 100.0)),
        moves,
        kept_for_capacity: kept,
        shortage,
    }
}

/// Whether the plans fall short of the demand, and the cheapest way to cover it: upgrading one
/// plan to a bigger tier, or adding one plan of the smallest tier that covers the shortfall (or
/// several of the largest, when none does). Ties go to the fix that adds more capacity.
fn shortage(provider: Provider, candidates: &[Candidate], capacity: f64, demand: Option<f64>, exhausted: usize) -> Option<Shortage> {
    let demand = demand?;
    let over = demand * (1.0 + POOL_BUFFER) - capacity;
    let limited = demand * (1.0 + 2.0 * POOL_BUFFER) - capacity;
    let (cause, need) = if over > 1e-9 { (ShortageCause::OverCapacity, over) }
        else if exhausted > 0 && limited > 1e-9 { (ShortageCause::HittingLimits, limited) }
        else { return None };
    let tiers = tiers(provider);
    let manage_url = billing_page(provider);
    let mut options: Vec<Fix> = vec![];
    for c in candidates {
        for (name, w, price) in tiers.iter().filter(|(_, w, _)| *w > c.weight && *w - c.weight + 1e-9 >= need) {
            let e = c.entry;
            options.push(Fix { action: FixAction::Upgrade, id: Some(e.id.clone()), email: e.email.clone(), alias: e.alias.clone(), from: Some(c.plan.clone()),
                to: name.to_string(), count: 1, monthly_cost: round((price - c.cost).max(0.0)), capacity_after: round(capacity + w - c.weight), manage_url });
        }
    }
    let add = |(name, w, price): &(&str, f64, f64), count: u32| Fix { action: FixAction::Add, id: None, email: None, alias: None, from: None,
        to: name.to_string(), count, monthly_cost: round(price * count as f64), capacity_after: round(capacity + w * count as f64), manage_url };
    match tiers.iter().filter(|(_, w, _)| *w + 1e-9 >= need).min_by(|a, b| a.2.total_cmp(&b.2)) {
        Some(t) => options.push(add(t, 1)),
        None => options.push(add(&tiers[0], (need / tiers[0].1 - 1e-9).ceil() as u32)),
    }
    let fix = options.into_iter().min_by(|a, b| a.monthly_cost.total_cmp(&b.monthly_cost).then_with(|| b.capacity_after.total_cmp(&a.capacity_after)))?;
    Some(Shortage { cause, shortfall: round(need), exhausted, fix })
}

fn new_move(c: &Candidate, action: Action, to: Option<String>, saving: f64) -> Move {
    let e = c.entry;
    Move {
        provider: e.provider, id: e.id.clone(), email: e.email.clone(), alias: e.alias.clone(), action,
        from: c.plan.clone(), to, monthly_saving: round(saving), verdict: e.assessment.verdict, basis: e.assessment.basis,
        peak_percent: e.recent_peak(), before: e.renewal.as_ref().map(|r| r.at), manage_url: billing_page(e.provider),
    }
}

fn round(v: f64) -> f64 { (v * 100.0).round() / 100.0 }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountUsage, Snapshot};
    use crate::plan_alerts::tests::{at, watched};
    use crate::settings::Settings;
    use crate::subscriptions::{report, History};
    use chrono::Duration;

    fn plan(provider: &str, id: &str, plan: &str, tier: Option<&str>, used: f64, now: DateTime<Utc>) -> AccountUsage {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": provider, "email": format!("{id}@example.invalid"), "alias": null, "plan": plan, "org": null,
            "active": false, "sources": [], "credits": null, "resets": [], "error": null, "stale": false, "fetched_at": now,
            "windows": [{ "kind": "weekly", "window_seconds": if provider == "codex" { Some(604800) } else { None }, "used_percent": used, "resets_at": now + Duration::days(3) }],
            "monitor_connected": true, "billing": { "tier": tier } })).unwrap()
    }

    /// Claude Max plans, each with four watched weeks peaking at the given share.
    fn claude(fleet: &[(&str, &str, f64)], now: DateTime<Utc>) -> Vec<ProviderAdvice> {
        let mut history = History::default();
        let accounts = fleet.iter().map(|(id, tier, peak)| {
            history.accounts.insert(format!("Claude:{id}"), watched(*peak, now));
            plan("claude", id, "max", Some(&format!("default_claude_max_{tier}")), 1.0, now)
        }).collect();
        report(&Snapshot { accounts, updated_at: Some(now) }, &history, &Settings::default(), None, now).advice
    }

    #[test]
    fn unused_plans_are_cancelled_and_underused_ones_downgraded_while_the_busiest_weeks_fit() {
        let now = at("2026-10-21T12:00:00Z");
        let a = &claude(&[("busy", "20x", 60.0), ("idle1", "20x", 0.0), ("idle2", "20x", 2.0), ("light", "20x", 15.0)], now)[0];
        assert_eq!((a.evidence, a.unit, a.capacity_now), (Evidence::Pool, "Max 20x", 4.0));
        // Busiest week 0.6 + 0.15 + 0.02 = 0.77 plans; floor 0.924.
        assert!((a.demand_p90.unwrap() - 0.77).abs() < 1e-9 && (a.floor.unwrap() - 0.92).abs() < 1e-9);
        let moves: Vec<(&str, Action, Option<&str>, f64)> = a.moves.iter().map(|m| (m.id.as_str(), m.action, m.to.as_deref(), m.monthly_saving)).collect();
        assert_eq!(moves, [("idle1", Action::Cancel, None, 200.0), ("idle2", Action::Cancel, None, 200.0), ("light", Action::Downgrade, Some("Max 5x"), 100.0)]);
        assert_eq!((a.monthly_saving, a.capacity_after, a.kept_for_capacity), (500.0, 1.25, 0));
        assert_eq!((a.spare_after, a.spare_after_percent), (Some(0.48), Some(62.34)));
        let m = &a.moves[0];
        assert_eq!((m.email.as_deref(), m.manage_url, m.verdict), (Some("idle1@example.invalid"), "https://claude.ai/settings/billing", Verdict::Unused));
    }

    #[test]
    fn advice_never_cuts_below_the_busiest_weeks_plus_twenty_percent() {
        let now = at("2026-10-21T12:00:00Z");
        // Busiest week 0.9 + 0.15 = 1.05 plans; floor 1.26. Two cancels leave 2; the downgrade would leave 1.25.
        let a = &claude(&[("busy", "20x", 90.0), ("idle1", "20x", 0.0), ("idle2", "20x", 0.0), ("light", "20x", 15.0)], now)[0];
        assert_eq!(a.moves.iter().map(|m| m.action).collect::<Vec<_>>(), [Action::Cancel, Action::Cancel]);
        assert_eq!((a.capacity_after, a.kept_for_capacity), (2.0, 1));
        // Whatever the fleet, what is left covers the floor, and one plan always stays.
        for busy in [0.0, 20.0, 50.0, 80.0, 99.0, 100.0] {
            for light in [3.0, 10.0, 20.0, 25.0] {
                let fleet = [("busy", "20x", busy), ("idle", "20x", 0.0), ("light", "20x", light), ("small", "5x", light), ("idle5", "5x", 0.0)];
                let a = &claude(&fleet, now)[0];
                assert!(a.capacity_after + 1e-9 >= a.floor.unwrap(), "{busy} {light}: {a:?}");
                assert!(a.moves.iter().filter(|m| m.action == Action::Cancel).count() < 5);
                for m in a.moves.iter().filter(|m| m.action == Action::Downgrade) {
                    let to = tiers(Provider::Claude).iter().find(|t| Some(t.0) == m.to.as_deref()).unwrap();
                    let from = tiers(Provider::Claude).iter().find(|t| t.0 == m.from).unwrap();
                    assert!(m.peak_percent.unwrap() / 100.0 * from.1 * 1.2 <= to.1 + 1e-9, "the new tier fits the busiest week: {m:?}");
                }
            }
        }
        // Every plan idle: all but one go.
        let a = &claude(&[("x", "20x", 0.0), ("y", "20x", 0.0), ("z", "20x", 0.0)], now)[0];
        assert_eq!((a.moves.len(), a.capacity_after, a.kept_for_capacity), (2, 1.0, 1));
    }

    #[test]
    fn a_shortage_gets_the_cheapest_fix_and_never_alongside_a_cut() {
        let now = at("2026-10-21T12:00:00Z");
        // Busiest weeks 2 plans; floor 2.4 over 2 held: add one Max 20x (no tier above to upgrade to).
        let a = &claude(&[("a", "20x", 100.0), ("b", "20x", 100.0)], now)[0];
        let s = a.shortage.as_ref().unwrap();
        assert_eq!((s.cause, s.shortfall, s.exhausted), (ShortageCause::OverCapacity, 0.4, 2));
        assert_eq!((s.fix.action, s.fix.to.as_str(), s.fix.count, s.fix.monthly_cost, s.fix.capacity_after), (FixAction::Add, "Max 20x", 1, 200.0, 3.0));
        assert!(a.moves.is_empty());

        // Short by 0.25: adding a Max 5x and upgrading the Max 5x to 20x both cost $100; the upgrade adds more.
        let a = &claude(&[("a", "20x", 100.0), ("b", "5x", 100.0)], now)[0];
        let f = &a.shortage.as_ref().unwrap().fix;
        assert_eq!((f.action, f.id.as_deref(), f.from.as_deref(), f.to.as_str(), f.monthly_cost, f.capacity_after), (FixAction::Upgrade, Some("b"), Some("Max 5x"), "Max 20x", 100.0, 2.0));
        assert_eq!(f.manage_url, "https://claude.ai/settings/billing");
        // Busiest weeks 1.05 plans, floor 1.26 over 1.25 held: a Pro plan is the cheapest cover.
        let a = &claude(&[("a", "20x", 80.0), ("b", "5x", 100.0)], now)[0];
        let s = a.shortage.as_ref().unwrap();
        assert_eq!((s.shortfall, s.fix.action, s.fix.to.as_str(), s.fix.monthly_cost), (0.01, FixAction::Add, "Pro", 20.0));

        // Within capacity but running into limits with little spare: real demand is cut short, so add.
        let a = &claude(&[("a", "20x", 100.0), ("b", "20x", 100.0), ("c", "20x", 50.0)], now)[0];
        let s = a.shortage.as_ref().unwrap();
        assert_eq!((s.cause, s.shortfall, s.fix.action, s.fix.monthly_cost), (ShortageCause::HittingLimits, 0.5, FixAction::Add, 200.0));
        // With room enough, plans at their limit are a matter of spreading the work: nothing to buy.
        assert!(claude(&[("a", "20x", 100.0), ("b", "20x", 100.0), ("idle", "20x", 0.0)], now)[0].shortage.is_none());
        assert!(claude(&[("a", "20x", 50.0), ("b", "20x", 50.0)], now)[0].shortage.is_none());
        // A cut and an addition never come together.
        for busy in [0.0, 50.0, 100.0] {
            for light in [0.0, 10.0, 100.0] {
                let a = &claude(&[("busy", "20x", busy), ("x", "20x", 100.0), ("idle", "20x", 0.0), ("light", "5x", light)], now)[0];
                assert!(a.moves.is_empty() || a.shortage.is_none(), "{busy} {light}: {a:?}");
            }
        }
    }

    #[test]
    fn busy_healthy_and_ending_plans_are_left_alone() {
        let now = at("2026-10-21T12:00:00Z");
        let a = &claude(&[("busy", "20x", 100.0), ("ok", "20x", 50.0)], now)[0];
        assert!(a.moves.is_empty() && a.monthly_saving == 0.0);
        let mut history = History::default();
        history.accounts.insert("Claude:gone".into(), watched(0.0, now));
        history.accounts.insert("Claude:busy".into(), watched(40.0, now));
        let mut gone = plan("claude", "gone", "max", Some("default_claude_max_20x"), 0.0, now);
        gone.billing.cancels_at = Some(now + Duration::days(4));
        let busy = plan("claude", "busy", "max", Some("default_claude_max_20x"), 0.0, now);
        let r = report(&Snapshot { accounts: vec![gone, busy], updated_at: Some(now) }, &history, &Settings::default(), None, now);
        assert!(r.advice[0].moves.is_empty(), "a plan already cancelled is not advised again");
        assert_eq!(r.advice[0].capacity_now, 1.0);
    }

    #[test]
    fn with_too_little_history_only_clear_evidence_counts() {
        let now = at("2026-10-21T12:00:00Z");
        let mut local = crate::codex_local::Local { attributed_since: Some(now - Duration::days(26)), ..Default::default() };
        local.accounts.insert("acct-busy".into(), crate::codex_local::Activity { threads: 5, tokens: 9000, last: Some(now - Duration::days(1)),
            days: [((now - Duration::days(1)).date_naive(), (5, 9000))].into_iter().collect() });
        // Idle all week: 0% with its reset a full week out.
        let mut idle = plan("codex", "u1:acct-idle", "pro", None, 0.0, now);
        idle.windows[0].resets_at = Some(now + Duration::days(7));
        let busy = plan("codex", "u2:acct-busy", "pro", None, 30.0, now);
        let r = report(&Snapshot { accounts: vec![idle.clone(), busy], updated_at: Some(now) }, &History::default(), &Settings::default(), Some(&local), now);
        let a = &r.advice[0];
        assert_eq!((a.evidence, a.weeks_used, a.demand_p90, a.unit), (Evidence::ClearOnly, 0, None, "Pro 200"));
        assert_eq!(a.moves.len(), 1);
        let m = &a.moves[0];
        assert_eq!((m.id.as_str(), m.action, m.monthly_saving, m.basis), ("u1:acct-idle", Action::Cancel, 200.0, Basis::LocalCodex));
        // This week's use + 20% still fits what is left: 0.3 plans of 1.
        assert_eq!((a.floor, a.capacity_after, a.spare_after), (Some(0.36), 1.0, Some(0.7)));
        // Alone, the idle plan is the provider's last: no advice to drop it.
        let r = report(&Snapshot { accounts: vec![idle], updated_at: Some(now) }, &History::default(), &Settings::default(), Some(&local), now);
        assert!(r.advice[0].moves.is_empty());
        // Underused on thin evidence is never downgraded.
        let mut history = History::default();
        let mut thin = watched(10.0, now);
        thin.windows.truncate(2);
        thin.first_seen = Some(now - Duration::days(2));
        history.accounts.insert("Claude:a".into(), thin.clone());
        history.accounts.insert("Claude:b".into(), thin);
        let accounts = vec![plan("claude", "a", "max", Some("default_claude_max_20x"), 1.0, now), plan("claude", "b", "max", Some("default_claude_max_20x"), 1.0, now)];
        let r = report(&Snapshot { accounts, updated_at: Some(now) }, &history, &Settings::default(), None, now);
        assert_eq!(r.accounts[0].assessment.verdict, Verdict::Underused);
        assert_eq!((r.advice[0].evidence, r.advice[0].moves.len()), (Evidence::ClearOnly, 0));
    }
}
