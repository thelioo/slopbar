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
    /// Claude's organization status ("active").
    pub status: Option<String>,
    /// When SlopBar last read the provider's account record (for the fields above).
    pub fetched_at: Option<DateTime<Utc>>,
}

impl BillingHints {
    /// These hints with every field the provider's own record answered taking precedence.
    pub fn overlay(mut self, provider: &BillingHints) -> BillingHints {
        macro_rules! take { ($($f:ident),*) => { $( if provider.$f.is_some() { self.$f = provider.$f.clone(); } )* } }
        take!(tier, billing_type, started_at, active_until, checked_at, renews_at, cancels_at, will_renew, delinquent, billing_period, status, fetched_at);
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
