//! In-process usage evidence. Provider strings, identities, bodies and headers cannot
//! be represented by this schema. Only explicitly named fields are inspected.
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    ClaudeProfile,
    ClaudeUsage,
    CodexUsage,
    CodexResetDetails,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    AccountMatches,
    OrganizationMatches,
    EmailMatches,
    FiveHour,
    SevenDay,
    SevenDayOpus,
    SevenDaySonnet,
    SevenDayFable,
    Utilization,
    ResetsAt,
    PrimaryWindow,
    SecondaryWindow,
    UsedPercent,
    WindowSeconds,
    ResetAt,
    CedarEmber,
    Grants,
    ResetsLeft,
    Paused,
    UsableNow,
    EndsAt,
    JuniperTide,
    Available,
    Eligible,
    NextAvailableAt,
    ResetCredits,
    AvailableCount,
    ApplicableAvailableCount,
    Credits,
    HasCredits,
    Unlimited,
    Balance,
    Status,
    ResetType,
    ExpiresAt,
    ExtraUsage,
    IsEnabled,
    MonthlyLimit,
    UsedCredits,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Missing,
    Null,
    Parsed,
    Invalid,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Known {
    Available,
    Consumed,
    Expired,
    Session,
    Weekly,
    Full,
    Unknown,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Probe {
    field: Field,
    parent: Option<Field>,
    index: Option<u16>,
    state: State,
    number: Option<f64>,
    boolean: Option<bool>,
    timestamp: Option<DateTime<Utc>>,
    known: Option<Known>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub endpoint: Endpoint,
    pub status: Option<u16>,
    pub parsed_json: bool,
    fields: Vec<Probe>,
    truncated: bool,
    unrecognized_window_count: u16,
}
pub type Observer<'a> = dyn Fn(Evidence) + Send + Sync + 'a;
enum Parse {
    Object,
    Array,
    Percent,
    Count,
    Bool,
    Time,
    Epoch,
    Balance,
    Known,
    Decimal,
}
impl Evidence {
    pub fn outcome(endpoint: Endpoint, status: Option<u16>) -> Self {
        Self {
            endpoint,
            status,
            parsed_json: false,
            fields: vec![],
            truncated: false,
            unrecognized_window_count: 0,
        }
    }
    fn probe(
        &mut self,
        field: Field,
        parent: Option<Field>,
        index: Option<u16>,
        value: Option<&Value>,
        parse: Parse,
    ) {
        let mut p = Probe {
            field,
            parent,
            index,
            state: State::Missing,
            number: None,
            boolean: None,
            timestamp: None,
            known: None,
        };
        if let Some(v) = value {
            p.state = if v.is_null() {
                State::Null
            } else {
                State::Invalid
            };
            if !v.is_null() {
                match parse {
                    Parse::Object if v.is_object() => p.state = State::Parsed,
                    Parse::Array if v.is_array() => {
                        p.number = Some(v.as_array().unwrap().len() as f64);
                        p.state = State::Parsed;
                    }
                    Parse::Percent => {
                        p.number = v
                            .as_f64()
                            .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
                    }
                    Parse::Count => {
                        p.number = v
                            .as_u64()
                            .filter(|n| *n <= u32::MAX as u64)
                            .map(|n| n as f64)
                    }
                    Parse::Bool => p.boolean = v.as_bool(),
                    Parse::Time => {
                        p.timestamp = v
                            .as_str()
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .map(|t| t.with_timezone(&Utc))
                    }
                    Parse::Epoch => {
                        p.timestamp = v.as_i64().and_then(|n| Utc.timestamp_opt(n, 0).single())
                    }
                    Parse::Balance => {
                        p.number = v
                            .as_str()
                            .and_then(|s| s.parse::<f64>().ok())
                            .filter(|n| n.is_finite())
                    }
                    Parse::Decimal => p.number = v.as_f64().filter(|n| n.is_finite()),
                    Parse::Known => {
                        p.known = Some(match v.as_str() {
                            Some("available") => Known::Available,
                            Some("consumed" | "used") => Known::Consumed,
                            Some("expired") => Known::Expired,
                            Some("session" | "five_hour") => Known::Session,
                            Some("weekly" | "seven_day") => Known::Weekly,
                            Some("full") => Known::Full,
                            _ => Known::Unknown,
                        })
                    }
                    _ => {}
                }
                if p.number.is_some()
                    || p.boolean.is_some()
                    || p.timestamp.is_some()
                    || p.known.is_some_and(|k| k != Known::Unknown)
                {
                    p.state = State::Parsed;
                }
            }
        }
        self.fields.push(p);
    }
}
fn get<'a>(v: &'a Value, snake: &str, camel: &str) -> Option<&'a Value> {
    match v.get(snake) {
        Some(x) if !x.is_null() => Some(x),
        x => v.get(camel).or(x),
    }
}
pub fn capture(endpoint: Endpoint, status: u16, raw: &Value) -> Evidence {
    use Field::*;
    let mut e = Evidence::outcome(endpoint, Some(status));
    e.parsed_json = true;
    let windows: Vec<(Field, &str, Option<&Value>)> = match endpoint {
        Endpoint::ClaudeUsage => [
            (FiveHour, "five_hour"),
            (SevenDay, "seven_day"),
            (SevenDayOpus, "seven_day_opus"),
            (SevenDaySonnet, "seven_day_sonnet"),
            (SevenDayFable, "seven_day_fable"),
        ]
        .into_iter()
        .map(|(f, key)| (f, key, raw.get(key)))
        .collect(),
        Endpoint::CodexUsage => [
            (PrimaryWindow, "primary_window"),
            (SecondaryWindow, "secondary_window"),
        ]
        .into_iter()
        .map(|(f, key)| (f, key, raw.get("rate_limit").and_then(|v| v.get(key))))
        .collect(),
        Endpoint::CodexResetDetails | Endpoint::ClaudeProfile => vec![],
    };
    for (f, _, w) in &windows {
        e.probe(*f, None, None, *w, Parse::Object);
        if let Some(w) = w.filter(|w| w.is_object()) {
            if endpoint == Endpoint::ClaudeUsage {
                e.probe(
                    Utilization,
                    Some(*f),
                    None,
                    w.get("utilization"),
                    Parse::Percent,
                );
                e.probe(ResetsAt, Some(*f), None, w.get("resets_at"), Parse::Time);
            } else {
                e.probe(
                    UsedPercent,
                    Some(*f),
                    None,
                    w.get("used_percent"),
                    Parse::Percent,
                );
                e.probe(
                    WindowSeconds,
                    Some(*f),
                    None,
                    w.get("limit_window_seconds"),
                    Parse::Count,
                );
                e.probe(ResetAt, Some(*f), None, w.get("reset_at"), Parse::Epoch);
            }
        }
    }
    if endpoint == Endpoint::ClaudeUsage {
        e.unrecognized_window_count = raw
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(k, v)| {
                        k.as_str() != "extra_usage"
                            && !windows.iter().any(|(_, key, _)| key == k)
                            && v.is_object()
                            && v.get("utilization").is_some()
                    })
                    .count()
                    .min(u16::MAX as usize) as u16
            })
            .unwrap_or(0);
        for (f, key) in [(CedarEmber, "cedar_ember"), (JuniperTide, "juniper_tide")] {
            e.probe(f, None, None, raw.get(key), Parse::Object);
        }
        let cedar = &raw["cedar_ember"];
        e.probe(
            Grants,
            Some(CedarEmber),
            None,
            cedar.get("grants"),
            Parse::Array,
        );
        if let Some(grants) = cedar.get("grants").and_then(Value::as_array) {
            e.truncated |= grants.len() > 64;
            for (i, g) in grants.iter().take(64).enumerate() {
                for (f, snake, camel, parse) in [
                    (ResetsLeft, "resets_left", "resetsLeft", Parse::Count),
                    (Paused, "paused", "paused", Parse::Bool),
                    (UsableNow, "usable_now", "usableNow", Parse::Bool),
                    (EndsAt, "ends_at", "endsAt", Parse::Time),
                ] {
                    e.probe(f, Some(Grants), Some(i as u16), get(g, snake, camel), parse);
                }
            }
        }
        let j = &raw["juniper_tide"];
        for (f, snake, camel, parse) in [
            (Available, "available", "available", Parse::Bool),
            (Eligible, "eligible", "eligible", Parse::Bool),
            (
                NextAvailableAt,
                "next_available_at",
                "nextAvailableAt",
                Parse::Time,
            ),
        ] {
            e.probe(f, Some(JuniperTide), None, get(j, snake, camel), parse);
        }
        e.probe(
            ExtraUsage,
            None,
            None,
            raw.get("extra_usage"),
            Parse::Object,
        );
        let credits = &raw["extra_usage"];
        for (f, key, parse) in [
            (IsEnabled, "is_enabled", Parse::Bool),
            (MonthlyLimit, "monthly_limit", Parse::Decimal),
            (UsedCredits, "used_credits", Parse::Decimal),
            (Utilization, "utilization", Parse::Percent),
        ] {
            e.probe(f, Some(ExtraUsage), None, credits.get(key), parse);
        }
    } else if endpoint == Endpoint::CodexUsage {
        e.probe(
            ResetCredits,
            None,
            None,
            raw.get("rate_limit_reset_credits"),
            Parse::Object,
        );
        let r = &raw["rate_limit_reset_credits"];
        for (f, key) in [
            (AvailableCount, "available_count"),
            (ApplicableAvailableCount, "applicable_available_count"),
        ] {
            e.probe(f, Some(ResetCredits), None, r.get(key), Parse::Count);
        }
        e.probe(Credits, None, None, raw.get("credits"), Parse::Object);
        let c = &raw["credits"];
        for (f, key, parse) in [
            (HasCredits, "has_credits", Parse::Bool),
            (Unlimited, "unlimited", Parse::Bool),
            (Balance, "balance", Parse::Balance),
        ] {
            e.probe(f, Some(Credits), None, c.get(key), parse);
        }
    } else {
        e.probe(Credits, None, None, raw.get("credits"), Parse::Array);
        if let Some(credits) = raw.get("credits").and_then(Value::as_array) {
            e.truncated |= credits.len() > 64;
            for (i, c) in credits.iter().take(64).enumerate() {
                for (f, key, parse) in [
                    (Status, "status", Parse::Known),
                    (ResetType, "reset_type", Parse::Known),
                    (ExpiresAt, "expires_at", Parse::Time),
                ] {
                    e.probe(f, Some(Credits), Some(i as u16), c.get(key), parse);
                }
            }
        }
    }
    e
}

/// Compare identities only in memory; the diagnostic schema can hold booleans,
/// never an email, organization name, UUID, profile body or token.
pub fn profile(status: u16, raw: &Value, slot: &crate::accounts::Slot) -> Evidence {
    let mut e = Evidence::outcome(Endpoint::ClaudeProfile, Some(status));
    e.parsed_json = true;
    let expected = &slot.data["oauth_account"];
    for (field, actual, expected) in [
        (Field::AccountMatches, raw["account"]["uuid"].as_str(), expected["accountUuid"].as_str()),
        (Field::OrganizationMatches, raw["organization"]["uuid"].as_str(), expected["organizationUuid"].as_str()),
        (Field::EmailMatches, raw["account"]["email"].as_str(), slot.email.as_deref()),
    ] {
        let comparison = actual.zip(expected).filter(|(a, b)| !a.is_empty() && !b.is_empty())
            .map(|(a, b)| Value::Bool(a == b));
        e.probe(field, None, None, comparison.as_ref(), Parse::Bool);
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extra_usage_is_credit_utilization_not_an_unrecognized_included_window() {
        let fixture: Value = serde_json::from_str(include_str!("fixtures/usage-divergence.json")).unwrap();
        let e = capture(Endpoint::ClaudeUsage, 200, &fixture["claude_usage"]);
        assert_eq!(e.unrecognized_window_count, 0);
        assert!(e.fields.iter().any(|p| p.field == Field::SevenDayFable && p.state == State::Missing));
        assert!(e.fields.iter().any(|p| p.field == Field::Utilization && p.parent == Some(Field::ExtraUsage) && p.number == Some(18.3675)));
    }

    #[test]
    fn profile_reports_matching_unknown_and_mismatched_identity_without_values() {
        let mut slot = crate::test_support::slot("PRIVATE-ID", "SECRET-access", "SECRET-refresh", 0);
        slot.email = Some("PRIVATE-email".into());
        slot.data["oauth_account"] = serde_json::json!({"accountUuid":"PRIVATE-account","organizationUuid":"PRIVATE-org"});
        let raw = serde_json::json!({"account":{"uuid":"PRIVATE-account","email":"PRIVATE-email"},"organization":{"uuid":"PRIVATE-other-org"},"credentials":"SECRET"});
        let e = profile(200, &raw, &slot);
        assert!(e.fields.iter().any(|p| p.field == Field::AccountMatches && p.boolean == Some(true)));
        assert!(e.fields.iter().any(|p| p.field == Field::OrganizationMatches && p.boolean == Some(false)));
        assert!(!serde_json::to_string(&e).unwrap().contains("PRIVATE"));
        assert!(!serde_json::to_string(&e).unwrap().contains("SECRET"));
        assert!(profile(200, &Value::Null, &slot).fields.iter().all(|p| p.state == State::Missing));
    }
    #[test]
    fn capture_is_allowlisted_and_distinguishes_absent_null_and_invalid() {
        let raw = serde_json::json!({"identity":"PRIVATE-CANARY", "authorization":"SECRET-CANARY", "cedar_ember":null,
            "juniper_tide":{"eligible":"SECRET-CANARY"}, "five_hour":{"utilization":10,"resets_at":"2026-10-13T05:00:00Z"},
            "seven_day_fable":{"utilization":3,"resets_at":"2026-10-19T04:00:00Z"}, "private_window":{"utilization":9}});
        let e = capture(Endpoint::ClaudeUsage, 200, &raw);
        let text = serde_json::to_string(&e).unwrap();
        for secret in [
            "PRIVATE-CANARY",
            "SECRET-CANARY",
            "private_window",
            "authorization",
            "identity",
        ] {
            assert!(!text.contains(secret));
        }
        assert!(e
            .fields
            .iter()
            .any(|p| p.field == Field::SevenDay && p.state == State::Missing));
        assert!(e
            .fields
            .iter()
            .any(|p| p.field == Field::CedarEmber && p.state == State::Null));
        assert!(e
            .fields
            .iter()
            .any(|p| p.field == Field::Eligible && p.state == State::Invalid));
        assert_eq!(e.unrecognized_window_count, 1);
    }
    #[test]
    fn detail_titles_and_unknown_enums_never_escape() {
        let e = capture(
            Endpoint::CodexResetDetails,
            200,
            &serde_json::json!({"credits":[{"title":"SECRET", "reset_type":"SECRET", "status":"available", "expires_at":"bad SECRET"}]}),
        );
        assert!(!serde_json::to_string(&e).unwrap().contains("SECRET"));
        assert!(e
            .fields
            .iter()
            .any(|p| p.field == Field::ResetType && p.known == Some(Known::Unknown)));
        assert!(e
            .fields
            .iter()
            .any(|p| p.field == Field::ExpiresAt && p.state == State::Invalid));
    }
}
