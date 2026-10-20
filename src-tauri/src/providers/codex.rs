//! Codex usage from a saved ChatGPT login.

use chrono::{TimeZone, Utc};
use serde::Deserialize;

use crate::model::{ResetGrant, UsageWindow};

#[derive(Deserialize)]
struct Usage {
    rate_limit: Option<RateLimit>,
    credits: Option<Credits>,
    rate_limit_reset_credits: Option<ResetCredits>,
}

/// Codex lets an account wipe its usage limit with a reset credit.
#[derive(Deserialize)]
struct ResetCredits {
    available_count: Option<u32>,
    /// How many can be used right now (typically only once a limit is reached).
    applicable_available_count: Option<u32>,
}

#[derive(Deserialize)]
struct RateLimit {
    primary_window: Option<Window>,
    secondary_window: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: f64,
    limit_window_seconds: Option<i64>,
    reset_at: Option<i64>,
}

#[derive(Deserialize)]
struct Credits {
    has_credits: Option<bool>,
    unlimited: Option<bool>,
    balance: Option<String>,
}

pub struct CodexUsage {
    pub windows: Vec<UsageWindow>,
    pub credits: Option<String>,
    pub resets: Vec<ResetGrant>,
}

#[derive(Deserialize)]
struct CreditList {
    credits: Vec<Credit>,
}

#[derive(Deserialize)]
struct Credit {
    status: Option<String>,
    title: Option<String>,
    reset_type: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
}

/// Fetches the usage windows, or an error code (see `model::AccountUsage::error`).
pub async fn fetch(client: &reqwest::Client, token: &str, account_id: Option<&str>, observe: &super::evidence::Observer<'_>) -> Result<CodexUsage, String> {
    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/usage")
        .bearer_auth(token)
        .header("User-Agent", "codex-cli");
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status() == 401 {
        return Err("token_invalid".into());
    }
    if resp.status() == 429 {
        return Err(super::rate_limited(&resp));
    }
    if !resp.status().is_success() {
        return Err(format!("http_{}", resp.status().as_u16()));
    }
    let status = resp.status().as_u16();
    let raw: serde_json::Value = match resp.json().await {
        Ok(raw) => raw,
        Err(_) => { observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::CodexUsage, Some(status))); return Err("bad_response".into()); }
    };
    observe(super::evidence::capture(super::evidence::Endpoint::CodexUsage, status, &raw));
    let usage: Usage = serde_json::from_value(raw).map_err(|_| "bad_response".to_string())?;
    let windows = usage
        .rate_limit
        .map(|rl| {
            [rl.primary_window, rl.secondary_window]
                .into_iter()
                .flatten()
                .map(|w| UsageWindow {
                    kind: window_kind(w.limit_window_seconds).into(),
                    window_seconds: w.limit_window_seconds,
                    used_percent: w.used_percent,
                    resets_at: w.reset_at.and_then(|t| Utc.timestamp_opt(t, 0).single()),
                })
                .collect()
        })
        .unwrap_or_default();
    let credits = usage.credits.and_then(|c| {
        if c.unlimited == Some(true) {
            Some("unlimited".into())
        } else if c.has_credits == Some(true) {
            c.balance
        } else {
            None
        }
    });
    // The usage payload only counts resets; fetch their details when there are any.
    let counts = usage.rate_limit_reset_credits;
    let available = counts.as_ref().and_then(|r| r.available_count).unwrap_or(0);
    let usable_now = counts.as_ref().and_then(|r| r.applicable_available_count).unwrap_or(0) > 0;
    let resets = if available > 0 {
        list_resets(client, token, account_id, usable_now, observe).await.unwrap_or_else(|| {
            vec![ResetGrant {
                kind: "unknown".into(),
                title: None,
                count: available,
                usable: usable_now,
                expires_at: None,
                next_available_at: None,
            }]
        })
    } else {
        vec![]
    };
    Ok(CodexUsage { windows, credits, resets })
}

/// Details of the account's reset credits: title and expiry of each available one.
async fn list_resets(
    client: &reqwest::Client,
    token: &str,
    account_id: Option<&str>,
    usable: bool,
    observe: &super::evidence::Observer<'_>,
) -> Option<Vec<ResetGrant>> {
    let mut req = client
        .get("https://chatgpt.com/backend-api/wham/rate-limit-reset-credits")
        .bearer_auth(token)
        .header("User-Agent", "codex-cli");
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let resp = match req.send().await {
        Ok(resp) => resp,
        Err(_) => { observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::CodexResetDetails, None)); return None; }
    };
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::CodexResetDetails, Some(status)));
        return None;
    }
    let raw: serde_json::Value = match resp.json().await {
        Ok(raw) => raw,
        Err(_) => { observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::CodexResetDetails, Some(status))); return None; }
    };
    observe(super::evidence::capture(super::evidence::Endpoint::CodexResetDetails, status, &raw));
    let list: CreditList = serde_json::from_value(raw).ok()?;
    Some(reset_grants(list, usable))
}

fn reset_grants(list: CreditList, usable: bool) -> Vec<ResetGrant> {
        list.credits
            .into_iter()
            .filter(|c| c.status.as_deref() == Some("available"))
            .map(|c| ResetGrant {
                kind: reset_kind(c.reset_type.as_deref()).into(),
                title: c.title,
                count: 1,
                usable,
                expires_at: c.expires_at,
                next_available_at: None,
            })
            .collect()
}

fn reset_kind(kind: Option<&str>) -> &'static str {
    match kind {
        Some("session" | "five_hour") => "session",
        Some("weekly" | "seven_day") => "weekly",
        Some("full") => "full",
        _ => "unknown",
    }
}

fn window_kind(seconds: Option<i64>) -> &'static str {
    match seconds {
        Some(18_000) => "session",
        Some(604_800) => "weekly",
        _ => "window",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_resets_with_unrecognized_types_keep_counts_not_assumed_scope() {
        let raw: serde_json::Value = serde_json::from_str(include_str!("fixtures/usage-divergence.json")).unwrap();
        let list: CreditList = serde_json::from_value(raw["codex_reset_details"].clone()).unwrap();
        let grants = reset_grants(list, true);
        assert_eq!(grants.iter().map(|r| r.count).sum::<u32>(), 2);
        assert!(grants.iter().all(|r| r.kind == "unknown" && r.usable && r.expires_at.is_some()));
        let usage: Usage = serde_json::from_value(raw["codex_usage"].clone()).unwrap();
        let counts = usage.rate_limit_reset_credits.unwrap();
        assert_eq!(counts.available_count, Some(2));
        assert_eq!(counts.applicable_available_count, Some(2));
        assert_eq!(usage.rate_limit.unwrap().primary_window.unwrap().used_percent, 100.0);
    }

    #[test]
    fn only_explicit_reset_scopes_are_recognized() {
        assert_eq!(reset_kind(Some("full")), "full");
        assert_eq!(reset_kind(Some("five_hour")), "session");
        assert_eq!(reset_kind(Some("seven_day")), "weekly");
        assert_eq!(reset_kind(None), "unknown");
        assert_eq!(reset_kind(Some("not_a_session_reset")), "unknown");
    }
}
