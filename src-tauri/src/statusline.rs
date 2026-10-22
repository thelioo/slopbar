//! `slopbar-mcp --statusline`: one compact line for Claude Code's status line, whose `statusLine`
//! setting runs a command and shows what it prints. It asks the running app through the door for
//! the Claude account in use and prints, e.g., "work · 12% 5h · 42% wk · runs out Thu 3PM".
//!
//! Claude Code redraws the status line often, so this must never slow it down: an answer is reused
//! for a few seconds, the door gets a short deadline, and on any error nothing is printed.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

use crate::accounts::Provider;
use crate::door::Request;

/// How long the door may take, connecting included.
pub const DOOR_LIMIT: Duration = Duration::from_millis(120);
/// How long an answer (or a failure, as an empty line) is reused before asking again.
pub const FRESH: Duration = Duration::from_secs(15);
/// Longest account name shown; the rest of the line matters more.
const NAME_CHARS: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
struct Cached {
    at: DateTime<Utc>,
    line: String,
}

/// Prints the line, or nothing. Never fails: a status line has nowhere to show an error.
pub fn run() {
    use std::io::Write;
    let cache = crate::settings::app_dir().map(|d| d.join("statusline.json"));
    if let Some(line) = line(&crate::door::pipe_name(), cache.as_deref(), DOOR_LIMIT, Utc::now()) {
        // Without a console (the release build has none) stdout may be missing: ignore that.
        let _ = writeln!(std::io::stdout(), "{line}");
    }
}

/// The line to print: from `cache` while fresh, else from the app at `pipe` within `limit`.
pub fn line(pipe: &str, cache: Option<&Path>, limit: Duration, now: DateTime<Utc>) -> Option<String> {
    let cached = cache.and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Cached>(&b).ok());
    if let Some(c) = cached.filter(|c| c.at <= now && (now - c.at).to_std().is_ok_and(|age| age < FRESH)) {
        return Some(c.line).filter(|l| !l.is_empty());
    }
    let request = Request::GetHeadroom { provider: Some(Provider::Claude), id: None };
    let started = std::time::Instant::now();
    let reply = crate::door::call_within(pipe, &request, limit);
    // SLOPBAR_STATUSLINE_DEBUG=1 says on stderr why nothing was printed.
    if std::env::var_os("SLOPBAR_STATUSLINE_DEBUG").is_some() {
        eprintln!("door: {} after {:?}", reply.as_ref().err().map_or("ok", String::as_str), started.elapsed());
    }
    let line = reply.ok().and_then(|v| format(&v, &chrono::Local, now));
    if let Some(path) = cache {
        // A failure is remembered too, so a stuck app costs the deadline once per `FRESH`.
        remember(path, &Cached { at: now, line: line.clone().unwrap_or_default() });
    }
    line
}

/// Saves the answer without flushing it to disk: a lost cache only means one more door call, while
/// a flush can take a second on a busy disk, far longer than the status line may.
fn remember(path: &Path, entry: &Cached) {
    let Ok(bytes) = serde_json::to_vec(entry) else { return };
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    if std::fs::write(&temp, bytes).is_err() || std::fs::rename(&temp, path).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
}

/// "name · 12% 5h · 42% wk · runs out Thu 3PM" from a `GetHeadroom` reply, with times in `tz`.
pub fn format<Tz: TimeZone>(reply: &Value, tz: &Tz, now: DateTime<Utc>) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let acc = reply["accounts"].as_array()?.iter().find(|a| a["active"] == true)?;
    let windows = acc["windows"].as_array().filter(|w| !w.is_empty())?;
    let name: String = acc["label"].as_str().unwrap_or("Claude").chars().filter(|c| !c.is_control()).take(NAME_CHARS).collect();
    let mut parts = vec![if name.trim().is_empty() { "Claude".to_string() } else { name }];
    let window = |kind: &str| windows.iter().find(|w| w["kind"] == kind);
    let at = |w: &Value, key: &str| w[key].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|t| t.with_timezone(tz).format("%a %-I%p").to_string());
    for (kind, short) in [("session", "5h"), ("weekly", "wk")] {
        if let Some(used) = window(kind).and_then(|w| w["used_percent"].as_f64()) {
            parts.push(format!("{}% {short}", used.round().clamp(0.0, 100.0)));
        }
    }
    let soonest = windows.iter()
        .filter(|w| matches!(w["advice"].as_str(), Some("at_limit" | "will_run_out_before_reset")))
        .min_by_key(|w| if w["advice"] == "at_limit" { i64::MIN } else { w["limit_in_minutes"].as_i64().unwrap_or(i64::MAX) });
    match soonest {
        Some(w) if w["advice"] == "at_limit" => parts.push(match at(w, "resets_at") { Some(t) => format!("limit · back {t}"), None => "limit".into() }),
        Some(w) => parts.extend(at(w, "limit_at").map(|t| format!("runs out {t}"))),
        None => {}
    }
    if acc["stale"] == true || reply["updated_at"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).is_some_and(|t| now - t.with_timezone(&Utc) > chrono::Duration::hours(1)) {
        parts.push("stale".into());
    }
    Some(parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use serde_json::json;

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    fn reply(weekly_advice: &str) -> Value {
        json!({ "updated_at": "2026-10-21T12:00:00Z", "accounts": [{
            "provider": "claude", "id": "a1", "label": "work@example.invalid", "active": true, "stale": false, "error": null,
            "windows": [
                { "kind": "session", "used_percent": 12.4, "resets_at": "2026-10-21T14:00:00Z", "limit_at": null, "limit_in_minutes": null, "advice": "plenty" },
                { "kind": "weekly", "used_percent": 41.6, "resets_at": "2026-10-24T09:00:00Z", "limit_at": "2026-10-23T15:00:00Z", "limit_in_minutes": 3060, "advice": weekly_advice },
            ],
        }] })
    }

    #[test]
    fn formats_one_compact_line_in_local_time() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let now = at("2026-10-21T12:05:00Z");
        assert_eq!(format(&reply("will_run_out_before_reset"), &utc, now).unwrap(), "work@example.invalid · 12% 5h · 42% wk · runs out Fri 3PM");
        assert_eq!(format(&reply("plenty"), &utc, now).unwrap(), "work@example.invalid · 12% 5h · 42% wk");
        let tokyo = FixedOffset::east_opt(9 * 3600).unwrap();
        assert!(format(&reply("will_run_out_before_reset"), &tokyo, now).unwrap().ends_with("runs out Sat 12AM"));
        assert!(format(&reply("at_limit"), &utc, now).unwrap().ends_with("42% wk · limit · back Sat 9AM"));
        assert!(format(&reply("plenty"), &utc, at("2026-10-21T15:00:00Z")).unwrap().ends_with(" · stale"), "an old snapshot says so");
    }

    #[test]
    fn names_are_cleaned_and_odd_replies_print_nothing() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let now = at("2026-10-21T12:05:00Z");
        let mut odd = reply("plenty");
        odd["accounts"][0]["label"] = json!("evil\u{1b}[2J\nname-that-goes-on-and-on-and-on-forever");
        let line = format(&odd, &utc, now).unwrap();
        assert!(!line.chars().any(char::is_control) && line.starts_with("evil[2Jname-that-goes-on-and-on- ·"), "{line}");
        odd["accounts"][0]["label"] = Value::Null;
        assert!(format(&odd, &utc, now).unwrap().starts_with("Claude · "));
        odd["accounts"][0]["windows"] = json!([]);
        assert_eq!(format(&odd, &utc, now), None, "no reading yet");
        assert_eq!(format(&json!({ "accounts": [] }), &utc, now), None);
        assert_eq!(format(&json!("nonsense"), &utc, now), None);
    }

    fn test_pipe() -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        format!(r"\\.\pipe\slopbar-statusline-test-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    #[test]
    fn a_fresh_line_is_reused_and_a_missing_app_prints_nothing() {
        let dir = crate::test_support::TempDir::new();
        let cache = dir.0.join("statusline.json");
        let now = at("2026-10-21T12:00:00Z");
        std::fs::write(&cache, serde_json::to_vec(&Cached { at: now - chrono::Duration::seconds(5), line: "cached".into() }).unwrap()).unwrap();
        assert_eq!(line(&test_pipe(), Some(&cache), DOOR_LIMIT, now).as_deref(), Some("cached"), "no door call while fresh");
        let later = now + chrono::Duration::seconds(20);
        assert_eq!(line(&test_pipe(), Some(&cache), DOOR_LIMIT, later), None, "the app is not running");
        assert_eq!(line(&test_pipe(), Some(&cache), DOOR_LIMIT, later + chrono::Duration::seconds(1)), None, "the failure is reused, quietly");
        let stored: Cached = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
        assert_eq!((stored.at, stored.line.as_str()), (later, ""));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_silent_app_is_given_up_on_quickly() {
        let name = test_pipe();
        let server = tokio::net::windows::named_pipe::ServerOptions::new().first_pipe_instance(true).create(&name).unwrap();
        tokio::spawn(async move {
            let _ = server.connect().await;
            tokio::time::sleep(Duration::from_secs(10)).await;
            drop(server);
        });
        let started = std::time::Instant::now();
        let result = tokio::task::spawn_blocking(move || line(&name, None, DOOR_LIMIT, Utc::now())).await.unwrap();
        assert_eq!(result, None);
        assert!(started.elapsed() < Duration::from_millis(1000), "took {:?}", started.elapsed());
    }
}
