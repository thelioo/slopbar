//! Claude usage from a saved login's OAuth token.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use serde_json::Value;

use crate::model::{ExtraUsage, ResetGrant, UsageWindow};

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
    pub extra_usage: Option<ExtraUsage>,
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
    let extra_usage = extra_usage(&raw);
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
    Ok(ClaudeUsage { windows, resets, billing: plan, extra_usage })
}

/// Extra usage (`extra_usage`: is_enabled, monthly_limit, used_credits, utilization), read
/// leniently: any field may be missing, null or a numeric string, and the whole object may be
/// absent (None). Amounts come in cents and are returned in dollars; a missing utilization is
/// worked out from the two amounts.
pub(crate) fn extra_usage(raw: &Value) -> Option<ExtraUsage> {
    let e = raw.get("extra_usage").filter(|v| v.is_object())?;
    let number = |snake: &str, camel: &str| -> Option<f64> {
        let v = field(e, snake, camel);
        v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).filter(|n: &f64| n.is_finite() && *n >= 0.0)
    };
    let monthly_limit = number("monthly_limit", "monthlyLimit").map(|c| c / 100.0);
    let used = number("used_credits", "usedCredits").map(|c| c / 100.0);
    let utilization = number("utilization", "utilization")
        .or_else(|| used.zip(monthly_limit).filter(|(_, l)| *l > 0.0).map(|(u, l)| u / l * 100.0));
    let enabled = field(e, "is_enabled", "isEnabled").as_bool();
    if enabled.is_none() && monthly_limit.is_none() && used.is_none() && utilization.is_none() { return None; }
    Some(ExtraUsage { enabled, monthly_limit, used, utilization })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_usage_reads_dollars_and_tolerates_missing_fields() {
        let fixture: Value = serde_json::from_str(include_str!("fixtures/usage-divergence.json")).unwrap();
        let e = extra_usage(&fixture["claude_usage"]).unwrap();
        assert_eq!((e.enabled, e.monthly_limit, e.used, e.utilization), (Some(false), Some(400.0), Some(73.47), Some(18.3675)));
        assert!(!e.active());
        // Absent, null or not an object: nothing to show.
        for raw in [serde_json::json!({}), serde_json::json!({ "extra_usage": null }), serde_json::json!({ "extra_usage": 5 }), serde_json::json!({ "extra_usage": {} })] {
            assert_eq!(extra_usage(&raw), None, "{raw}");
        }
        // Utilization missing: worked out from the amounts. Strings and camelCase are read too.
        let e = extra_usage(&serde_json::json!({ "extra_usage": { "isEnabled": true, "monthly_limit": "20000", "usedCredits": 15000 } })).unwrap();
        assert_eq!((e.enabled, e.monthly_limit, e.used, e.utilization), (Some(true), Some(200.0), Some(150.0), Some(75.0)));
        // A limit of zero gives no utilization; nonsense numbers are dropped.
        let e = extra_usage(&serde_json::json!({ "extra_usage": { "is_enabled": true, "monthly_limit": 0, "used_credits": -3, "utilization": null } })).unwrap();
        assert_eq!((e.monthly_limit, e.used, e.utilization), (Some(0.0), None, None));
        let e = extra_usage(&serde_json::json!({ "extra_usage": { "is_enabled": true } })).unwrap();
        assert!(e.active() && e.monthly_limit.is_none());
    }
}
