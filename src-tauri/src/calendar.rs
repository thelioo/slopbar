//! Plan renewals as an iCalendar file (RFC 5545) that Outlook or Google Calendar can subscribe to:
//! one all-day event per paid plan's next renewal, with a reminder three days ahead for plans that
//! look unused or underused. Written to a path the user chooses (Documents\SlopBar\renewals.ics by
//! default) and rewritten after each refresh while turned on.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use crate::settings::Settings;
use crate::subscriptions::{DateSource, PlanKind, Report, Verdict, WASTEFUL_DAYS};

/// Longest path accepted from settings.
const MAX_PATH: usize = 1024;
/// RFC 5545 §3.1: content lines are folded after 75 octets.
const LINE_OCTETS: usize = 75;

/// Documents\SlopBar\renewals.ics.
pub fn default_path() -> Option<PathBuf> {
    dirs::document_dir().map(|d| d.join("SlopBar").join("renewals.ics"))
}

/// Where the calendar goes: the path in settings, else the default.
pub fn path(settings: &Settings) -> Option<PathBuf> {
    settings.calendar.path.as_deref().map(PathBuf::from).or_else(default_path)
}

/// Whether `path` can hold the calendar: an absolute `.ics` file path in an existing folder (or the
/// default path, whose folder is created).
pub fn valid_path(path: &str) -> bool {
    let p = Path::new(path);
    let shape = path.len() <= MAX_PATH && !path.chars().any(char::is_control) && p.is_absolute()
        && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ics"))
        && !p.components().any(|c| matches!(c, std::path::Component::ParentDir));
    shape && (p.parent().is_some_and(Path::is_dir) || default_path().is_some_and(|d| d == p))
}

/// One renewal in the calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub uid: String,
    pub date: NaiveDate,
    pub summary: String,
    pub description: String,
    /// Remind `WASTEFUL_DAYS` ahead: the plan looks unused or underused.
    pub alarm: bool,
}

/// The paid plans' next renewals, by date. A date the user entered is a calendar day; a moment
/// from the provider falls on its day in `tz`.
pub fn events<Tz: TimeZone>(report: &Report, tz: &Tz) -> Vec<Event> {
    let mut out: Vec<Event> = report.accounts.iter()
        .filter(|e| e.plan_kind != PlanKind::Free && e.ends_at.is_none())
        .filter_map(|e| {
            let renewal = e.renewal.as_ref()?;
            let date = match renewal.source {
                DateSource::Entered => renewal.at.date_naive(),
                _ => renewal.at.with_timezone(tz).date_naive(),
            };
            let provider = provider_name(e.provider);
            let name = e.alias.clone().or_else(|| e.email.clone()).unwrap_or_else(|| e.id.clone());
            let alarm = matches!(e.assessment.verdict, Verdict::Unused | Verdict::Underused);
            let mut description = vec![format!("{provider} {} plan", e.plan.as_deref().unwrap_or("paid"))];
            if let Some(cost) = e.monthly_cost { description.push(format!("${cost:.2}/month")); }
            match e.assessment.verdict {
                Verdict::Unused => description.push("Looks unused.".into()),
                Verdict::Underused => description.push("Looks underused.".into()),
                Verdict::ExhaustedWeekly => description.push("Runs out most weeks.".into()),
                _ => {}
            }
            if renewal.source == DateSource::Estimated { description.push("Date estimated from the plan's start.".into()); }
            if alarm { description.push(format!("Not needed? Cancel it on the provider's billing page: {}", crate::subscriptions::billing_page(e.provider))); }
            Some(Event {
                uid: format!("renewal-{}-{:016x}-{}@slopbar", e.provider.name().to_lowercase(), fnv(&e.id), date.format("%Y%m%d")),
                date,
                summary: format!("{provider} plan renews · {name}"),
                description: description.join("\n"),
                alarm,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.date, &a.summary, &a.uid).cmp(&(b.date, &b.summary, &b.uid)));
    out
}

fn provider_name(p: crate::accounts::Provider) -> &'static str {
    match p {
        crate::accounts::Provider::Claude => "Claude",
        crate::accounts::Provider::Codex => "Codex",
    }
}

/// FNV-1a: stable across builds and machines, so UIDs never change for the same account and date.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// The calendar file, with CRLF line ends and long lines folded.
pub fn render(events: &[Event], now: DateTime<Utc>) -> String {
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//SlopBar//Plan renewals//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        "X-WR-CALNAME:SlopBar plan renewals".into(),
        "REFRESH-INTERVAL;VALUE=DURATION:PT6H".into(),
        "X-PUBLISHED-TTL:PT6H".into(),
    ];
    for e in events {
        lines.extend([
            "BEGIN:VEVENT".to_string(),
            format!("UID:{}", e.uid),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART;VALUE=DATE:{}", e.date.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", e.date.succ_opt().unwrap_or(e.date).format("%Y%m%d")),
            format!("SUMMARY:{}", text(&e.summary)),
            format!("DESCRIPTION:{}", text(&e.description)),
            "TRANSP:TRANSPARENT".into(),
        ]);
        if e.alarm {
            lines.extend([
                "BEGIN:VALARM".to_string(),
                "ACTION:DISPLAY".into(),
                format!("DESCRIPTION:{}", text(&e.summary)),
                format!("TRIGGER:-P{WASTEFUL_DAYS}D"),
                "END:VALARM".into(),
            ]);
        }
        lines.push("END:VEVENT".into());
    }
    lines.push("END:VCALENDAR".into());
    lines.iter().map(|l| fold(l)).collect::<Vec<_>>().join("")
}

/// RFC 5545 §3.3.11 TEXT: backslash, semicolon and comma escaped, line breaks as `\n`, other
/// control characters dropped.
fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// One content line ending in CRLF, folded so no physical line exceeds 75 octets and no UTF-8
/// character is split.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > LINE_OCTETS {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += c.len_utf8();
    }
    out.push_str("\r\n");
    out
}

/// Writes `content` to `path` (creating its folder), unless only the timestamps changed, so a
/// subscribed calendar is not rewritten every few minutes. Returns whether the file changed.
pub fn write(path: &Path, content: &str) -> std::io::Result<bool> {
    let without_stamps = |s: &str| s.lines().filter(|l| !l.starts_with("DTSTAMP:")).collect::<Vec<_>>().join("\n");
    if std::fs::read_to_string(path).is_ok_and(|old| without_stamps(&old) == without_stamps(content)) {
        return Ok(false);
    }
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    std::fs::create_dir_all(dir)?;
    // A calendar app reading mid-write sees the old file or the new one, never half of one.
    let temp = dir.join(format!(".{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("renewals.ics")));
    std::fs::write(&temp, content)?;
    std::fs::rename(&temp, path).inspect_err(|_| { let _ = std::fs::remove_file(&temp); })?;
    Ok(true)
}

/// Builds and writes the calendar for `report` to the path in `settings`. Returns the path and the
/// number of renewals in it.
pub fn export(report: &Report, settings: &Settings, now: DateTime<Utc>) -> Result<(PathBuf, usize), String> {
    let path = path(settings).ok_or("no_documents_folder")?;
    let events = events(report, &chrono::Local);
    write(&path, &render(&events, now)).map_err(|_| "calendar_unwritable".to_string())?;
    Ok((path, events.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_alerts::tests::{at, world};
    use crate::subscriptions::report;
    use crate::test_support::TempDir;
    use chrono::{Duration, FixedOffset};

    fn calendar(now: DateTime<Utc>, tz: &FixedOffset) -> Vec<Event> {
        let w = world(now);
        events(&report(&w.snapshot, &w.history, &w.settings, None, now), tz)
    }

    #[test]
    fn one_event_per_paid_renewal_with_a_reminder_for_unneeded_plans() {
        let now = at("2026-10-21T09:00:00Z");
        // West of UTC: an entered date stays the day the user picked.
        let events = calendar(now, &FixedOffset::west_opt(8 * 3600).unwrap());
        let names: Vec<_> = events.iter().map(|e| (e.summary.as_str(), e.date.to_string(), e.alarm)).collect();
        assert_eq!(names, [("Codex plan renews · busy@example.invalid", "2026-10-24".into(), false), ("Codex plan renews · idle@example.invalid", "2026-10-24".into(), true)],
            "the cancelled plan has no renewal");
        assert!(events[1].description.contains("Looks unused") && events[1].description.contains("https://"));
    }

    #[test]
    fn uids_are_stable_per_account_and_date() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let first = calendar(at("2026-10-21T09:00:00Z"), &tz);
        let again = calendar(at("2026-10-22T18:00:00Z"), &tz);
        assert_eq!(first.iter().map(|e| &e.uid).collect::<Vec<_>>(), again.iter().map(|e| &e.uid).collect::<Vec<_>>());
        assert_ne!(first[0].uid, first[1].uid);
        assert!(first.iter().all(|e| e.uid.ends_with("-20261024@slopbar") && !e.uid.contains("busy")));
        assert!(!first.iter().any(|e| e.uid.contains("example.invalid")), "no email in the UID");
        // Next month's renewal is a new event.
        let mut w = world(at("2026-11-21T09:00:00Z"));
        for s in w.settings.subscriptions.values_mut() { s.next_billing = Some("2026-11-24".parse().unwrap()); }
        w.snapshot.accounts[2].billing.cancels_at = Some(at("2026-11-24T00:00:00Z"));
        let next = events(&report(&w.snapshot, &w.history, &w.settings, None, at("2026-11-21T09:00:00Z")), &tz);
        assert!(next.iter().all(|e| e.uid.ends_with("-20261124@slopbar")) && next[0].uid != first[0].uid);
    }

    #[test]
    fn renders_valid_folded_escaped_icalendar() {
        let event = Event { uid: "renewal-claude-1-20261103@slopbar".into(), date: "2026-11-03".parse().unwrap(),
            summary: "Claude plan renews · Ünïcode, work; \\home".into(), description: format!("line one\nline two{}", " é".repeat(60)), alarm: true };
        let ics = render(&[event], at("2026-10-21T09:30:00Z"));
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n") && ics.ends_with("END:VCALENDAR\r\n"));
        assert!(!ics.replace("\r\n", "").contains('\n'), "only CRLF line ends");
        for line in ics.split("\r\n") { assert!(line.len() <= 75, "{} octets: {line}", line.len()); }
        let unfolded = ics.replace("\r\n ", "");
        assert!(unfolded.contains("SUMMARY:Claude plan renews · Ünïcode\\, work\\; \\\\home\r\n"));
        assert!(unfolded.contains("DESCRIPTION:line one\\nline two é é"));
        assert!(unfolded.contains("DTSTART;VALUE=DATE:20261103\r\nDTEND;VALUE=DATE:20261104\r\n"));
        assert!(unfolded.contains("DTSTAMP:20261021T093000Z\r\n"));
        assert!(unfolded.contains("BEGIN:VALARM\r\nACTION:DISPLAY\r\n") && unfolded.contains("TRIGGER:-P3D\r\nEND:VALARM\r\nEND:VEVENT\r\n"));
        let opens = |w: &str| unfolded.matches(&format!("BEGIN:{w}\r\n")).count() == unfolded.matches(&format!("END:{w}\r\n")).count();
        assert!(opens("VCALENDAR") && opens("VEVENT") && opens("VALARM"));
        assert_eq!(text("a\u{7}b\r\nc"), "ab\\nc", "other control characters are dropped");
    }

    #[test]
    fn rewrites_only_when_more_than_the_timestamp_changed() {
        let dir = TempDir::new();
        let path = dir.0.join("nested").join("renewals.ics");
        let tz = FixedOffset::east_opt(0).unwrap();
        let now = at("2026-10-21T09:00:00Z");
        let events = calendar(now, &tz);
        assert!(write(&path, &render(&events, now)).unwrap(), "first write creates the folder");
        assert!(!write(&path, &render(&events, now + Duration::minutes(5))).unwrap());
        assert!(write(&path, &render(&events[..1], now)).unwrap());
        assert!(std::fs::read_to_string(&path).unwrap().contains("BEGIN:VEVENT"));
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1, "no temp file left");
    }

    #[test]
    fn only_absolute_ics_paths_in_existing_folders_are_accepted() {
        let dir = TempDir::new();
        let ok = dir.0.join("renewals.ics");
        assert!(valid_path(ok.to_str().unwrap()));
        assert!(valid_path(dir.0.join("R.ICS").to_str().unwrap()));
        for bad in [dir.0.join("renewals.txt"), dir.0.join("missing").join("r.ics"), dir.0.join("..").join("r.ics"), PathBuf::from("relative.ics")] {
            assert!(!valid_path(bad.to_str().unwrap()), "{bad:?}");
        }
        assert!(!valid_path(&format!("{}\n.ics", dir.0.display())));
    }
}
