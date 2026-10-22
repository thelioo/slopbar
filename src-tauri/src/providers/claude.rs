//! Claude usage from a saved login's OAuth token.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use serde_json::Value;

use crate::model::{ResetGrant, UsageWindow};

#[derive(Deserialize)]
struct Usage {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
    seven_day_opus: Option<Window>,
    seven_day_sonnet: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    utilization: Option<f64>,
    resets_at: Option<DateTime<Utc>>,
}

pub struct ClaudeUsage {
    pub windows: Vec<UsageWindow>,
    pub resets: Vec<ResetGrant>,
    pub billing: Option<crate::model::BillingHints>,
}

/// The plan facts in the profile: when the subscription started, its status and tier. Claude
/// exposes no renewal date or cancellation to this token.
fn billing(raw: &Value) -> crate::model::BillingHints {
    let org = &raw["organization"];
    let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(String::from);
    crate::model::BillingHints {
        tier: text(&org["rate_limit_tier"]),
        billing_type: text(&org["billing_type"]),
        started_at: org["subscription_created_at"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|t| t.with_timezone(&Utc)),
        status: text(&org["subscription_status"]),
        fetched_at: Some(Utc::now()),
        ..Default::default()
    }
}

/// Fetches the usage windows, or an error code (see `model::AccountUsage::error`).
pub async fn fetch(client: &reqwest::Client, token: &str, slot: &crate::accounts::Slot, observe: &super::evidence::Observer<'_>) -> Result<ClaudeUsage, String> {
    // Read-only identity evidence and plan facts every twelve hours per account, using the same
    // access token the monitor supplied. Never refresh, replace or persist a login here.
    let mut plan = None;
    if super::billing_due(&format!("claude:{}", slot.id)) {
        let response = client.get("https://api.anthropic.com/api/oauth/profile")
            .bearer_auth(token).header("anthropic-beta", "oauth-2025-04-20").send().await;
        match response {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if status == 429 {
                    observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::ClaudeProfile, Some(status)));
                    return Err(super::rate_limited(&resp));
                }
                if resp.status().is_success() {
                    match resp.json::<Value>().await {
                        Ok(raw) => { plan = Some(billing(&raw)); observe(super::evidence::profile(status, &raw, slot)) }
                        Err(_) => observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::ClaudeProfile, Some(status))),
                    }
                } else { observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::ClaudeProfile, Some(status))); }
            }
            Err(_) => observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::ClaudeProfile, None)),
        }
    }
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(token)
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|e| e.to_string())?;
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
    let raw: Value = match resp.json().await {
        Ok(raw) => raw,
        Err(_) => { observe(super::evidence::Evidence::outcome(super::evidence::Endpoint::ClaudeUsage, Some(status))); return Err("bad_response".into()); }
    };
    observe(super::evidence::capture(super::evidence::Endpoint::ClaudeUsage, status, &raw));
    let resets = resets(&raw);
    let u: Usage = serde_json::from_value(raw).map_err(|_| "bad_response".to_string())?;
    let windows = [
        ("session", u.five_hour),
        ("weekly", u.seven_day),
        ("weekly_opus", u.seven_day_opus),
        ("weekly_sonnet", u.seven_day_sonnet),
    ];
    let windows = windows
        .into_iter()
        .filter_map(|(kind, w)| {
            let w = w?;
            Some(UsageWindow {
                kind: kind.into(),
                window_seconds: None,
                used_percent: w.utilization?,
                resets_at: w.resets_at,
            })
        })
        .collect();
    Ok(ClaudeUsage { windows, resets, billing: plan })
}

/// A field under either of its spellings (the API is snake_case; be lenient).
fn field<'a>(v: &'a Value, snake: &str, camel: &str) -> &'a Value {
    match v.get(snake) {
        Some(x) if !x.is_null() => x,
        _ => v.get(camel).unwrap_or(&Value::Null),
    }
}

fn time(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
}

/// Claude's usage-limit resets. They ride along in the usage payload under internal names:
/// `cedar_ember` holds granted resets (each grant has resets left and an end date) and
/// `juniper_tide` a session-limit reset that comes back periodically. Both are null when the
/// account has none. Read leniently: these fields are undocumented.
fn resets(raw: &Value) -> Vec<ResetGrant> {
    let mut out = Vec::new();
    let cedar = &raw["cedar_ember"];
    if let Some(grants) = cedar.get("grants").and_then(Value::as_array) {
        for g in grants {
            let left = field(g, "resets_left", "resetsLeft").as_u64().unwrap_or(0) as u32;
            if left == 0 || field(g, "paused", "paused").as_bool() == Some(true) {
                continue;
            }
            out.push(ResetGrant {
                kind: "full".into(),
                title: None,
                count: left,
                usable: field(g, "usable_now", "usableNow").as_bool().unwrap_or(false),
                expires_at: time(field(g, "ends_at", "endsAt")),
                next_available_at: None,
            });
        }
    }
    let juniper = &raw["juniper_tide"];
    if juniper.is_object() {
        let next = time(field(juniper, "next_available_at", "nextAvailableAt"));
        let available = field(juniper, "available", "available").as_bool()
            .or(field(juniper, "eligible", "eligible").as_bool())
            .unwrap_or(next.is_none());
        if available || next.is_some() {
            out.push(ResetGrant {
                kind: "session".into(),
                title: None,
                count: u32::from(available),
                usable: available,
                expires_at: None,
                next_available_at: next,
            });
        }
    }
    out
}
