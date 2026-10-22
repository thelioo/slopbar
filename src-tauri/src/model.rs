use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::accounts::Provider;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UsageWindow {
    /// "session", "weekly", "weekly_opus", "weekly_sonnet" or "window"; translated by the frontend.
    pub kind: String,
    pub window_seconds: Option<i64>,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountUsage {
    /// Vault slot id; stable across token refreshes.
    pub id: String,
    pub provider: Provider,
    pub email: Option<String>,
    pub alias: Option<String>,
    pub plan: Option<String>,
    pub org: Option<String>,
    /// Whether this account is the one signed in to the CLI.
    pub active: bool,
    /// Where it is signed in, e.g. "Windows" or "WSL: Ubuntu".
    pub sources: Vec<String>,
    pub windows: Vec<UsageWindow>,
    /// A balance, or "unlimited".
    pub credits: Option<String>,
    /// Limit resets the account holds (granted by Anthropic/OpenAI, used on demand).
    pub resets: Vec<ResetGrant>,
    /// An error code ("token_expired", "token_invalid", "bad_response", "http_<status>")
    /// or a raw network error message.
    pub error: Option<String>,
    /// Independent monitoring can need recovery while live CLI usage is fresh.
    #[serde(default)]
    pub monitor_error: Option<String>,
    /// Last observed numbers; never predict a reset without a successful provider response.
    pub stale: bool,
    pub fetched_at: Option<DateTime<Utc>>,
    pub monitor_connected: bool,
    /// What the saved login says about the plan's billing (see `Slot::billing`).
    #[serde(default)]
    pub billing: BillingHints,
    /// Other machines that say (through the shared presence folder) this account is signed in
    /// there. `active` and `sources` stay about this machine only.
    #[serde(default)]
    pub remote: Vec<RemoteSeat>,
    /// Usage kept rising while the account was signed in nowhere SlopBar can see: another
    /// machine, the web or the desktop app.
    #[serde(default)]
    pub used_outside: bool,
    /// When the weekly limit runs out at the current rate, when that is before it resets
    /// (`subscriptions::forecast`). Never set from a stale reading.
    #[serde(default)]
    pub runs_out_at: Option<DateTime<Utc>>,
    /// Use seen while none of the user's computers had it signed in (see `unexpected`), when
    /// there was any this week.
    #[serde(default)]
    pub unexpected: Option<UnexpectedUse>,
    /// The provider rejected the CLI's own saved login (its access token got a 401). Cleared as
    /// soon as that login is renewed or works again; the CLI signs in again on its own.
    #[serde(default)]
    pub cli_rejected: bool,
    /// Claude: pay-as-you-go use beyond the plan's limits this month, when the usage endpoint
    /// reported it (see `providers::claude::extra_usage`).
    #[serde(default)]
    pub extra_usage: Option<ExtraUsage>,
    /// The purpose of the lease reserving this account for a job (see `leases`), set on the
    /// snapshot the app publishes.
    #[serde(default)]
    pub leased: Option<String>,
}

/// Claude's extra usage: what the account may spend beyond its plan each month, and what it has.
/// Amounts are in US dollars; the endpoint reports cents (a 40000 limit is $400).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ExtraUsage {
    /// The user turned extra usage on. None when the endpoint did not say.
    pub enabled: Option<bool>,
    pub monthly_limit: Option<f64>,
    pub used: Option<f64>,
    /// Used against the limit, in percent.
    pub utilization: Option<f64>,
}

impl ExtraUsage {
    /// Turned on: it can cost money this month.
    pub fn active(&self) -> bool { self.enabled == Some(true) }
}

/// A monitored account's weekly usage jumped while every computer sharing its status said it
/// was not signed in there: the web or mobile app, or someone else with the login.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UnexpectedUse {
    /// Separate occasions in the last seven days.
    pub seen_week: u32,
    pub last_seen: DateTime<Utc>,
    /// When the user was last notified about it (at most once every six hours); a new value
    /// means a new notification is due.
    pub alert_at: Option<DateTime<Utc>>,
}

/// Where another machine has an account signed in.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteSeat {
    /// That machine's name.
    pub machine: String,
    /// Where on it, e.g. "Windows" or "WSL: Ubuntu".
    #[serde(rename = "where")]
    pub where_: Vec<String>,
    /// When it last said so.
    pub seen_at: DateTime<Utc>,
    /// It is still writing; otherwise it went quiet within the last day.
    pub live: bool,
}

/// Plan and billing facts from the saved login and, when SlopBar asked the provider, its own
/// account record. None of them is a promise: token claims are only as current as the token, so
/// the report labels every date with where it came from.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct BillingHints {
    /// Claude's rate-limit tier ("default_claude_max_20x"), which tells Max 5x from 20x.
    pub tier: Option<String>,
    /// How the plan is paid ("stripe_subscription"; app stores bill differently).
    pub billing_type: Option<String>,
    /// When the subscription started (Claude) or its current period began (Codex).
    pub started_at: Option<DateTime<Utc>>,
    /// Codex: the end of the paid period, as of `checked_at`.
    pub active_until: Option<DateTime<Utc>>,
    pub checked_at: Option<DateTime<Utc>>,
    /// Codex's account record: the next charge, and the day a cancelled plan ends.
    pub renews_at: Option<DateTime<Utc>>,
    pub cancels_at: Option<DateTime<Utc>>,
    pub will_renew: Option<bool>,
    /// A payment failed and the provider is waiting for it.
    pub delinquent: Option<bool>,
    /// "monthly" or "yearly".
    pub billing_period: Option<String>,
    /// The provider's own name for the plan bought ("chatgptpro"), finer than the plan type.
    pub plan_code: Option<String>,
    /// Codex: the plan type the usage endpoint reports on every poll ("prolite", "pro", "promax"
    /// for Pro 100/200/500). Fresher than the token's claim, which lags a plan change until the
    /// token is reissued.
    pub plan_type: Option<String>,
    /// Claude's organization status ("active").
    pub status: Option<String>,
    /// When SlopBar last read the provider's account record (for the fields above).
    pub fetched_at: Option<DateTime<Utc>>,
}

impl BillingHints {
    /// These hints with every field the provider's own record answered taking precedence.
    pub fn overlay(mut self, provider: &BillingHints) -> BillingHints {
        macro_rules! take { ($($f:ident),*) => { $( if provider.$f.is_some() { self.$f = provider.$f.clone(); } )* } }
        take!(tier, billing_type, started_at, active_until, checked_at, renews_at, cancels_at, will_renew, delinquent, billing_period, plan_code, plan_type, status, fetched_at);
        self
    }
}

impl AccountUsage {
    /// The highest usage across windows: what the balancer compares against its threshold.
    pub fn peak(&self) -> Option<f64> {
        self.windows.iter().map(|w| w.used_percent).reduce(f64::max)
    }
}

/// A usage-limit reset the account can spend to refill its limits.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResetGrant {
    /// "full", "session", "weekly", or "unknown" when the provider scope is unrecognized.
    pub kind: String,
    /// The provider's own title, when it gives one (e.g. "Full reset").
    pub title: Option<String>,
    pub count: u32,
    /// Whether it can be spent right now (usually only once a limit is reached).
    pub usable: bool,
    pub expires_at: Option<DateTime<Utc>>,
    /// For resets that come back periodically: when the next one becomes available.
    pub next_available_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub accounts: Vec<AccountUsage>,
    pub updated_at: Option<DateTime<Utc>>,
}
