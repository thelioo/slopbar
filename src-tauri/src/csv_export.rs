//! CSV files of what SlopBar knows, for a spreadsheet: every account's completed weekly windows
//! (`usage-history.json`), the usage breakdown by day, account, repository and model
//! (`usage_index`), and the subscription report. Written only where the user (or an agent the
//! user asked) points, never uploaded.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::subscriptions::{History, Report};
use crate::usage_index::{DailyRow, Known};

/// Documents\SlopBar Exports.
pub fn default_folder() -> Option<PathBuf> {
    dirs::document_dir().or_else(|| dirs::home_dir().map(|h| h.join("Documents"))).map(|d| d.join("SlopBar Exports"))
}

/// A text cell: quoted when it must be, and never read as a formula by a spreadsheet (emails,
/// aliases and folder names are written by others).
fn text(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control() || *c == '\t').collect();
    let s = if s.starts_with(['=', '+', '-', '@', '\t']) { format!("'{s}") } else { s };
    if s.contains([',', '"', '\n']) || s.starts_with(' ') || s.ends_with(' ') { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

fn opt(s: Option<&str>) -> String { s.map(text).unwrap_or_default() }
fn time(t: Option<DateTime<Utc>>) -> String { t.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default() }
fn num(x: Option<f64>) -> String { x.filter(|x| x.is_finite()).map(|x| format!("{}", (x * 100.0).round() / 100.0)).unwrap_or_default() }
/// An enum's serialized name.
fn name(x: impl Serialize) -> String { serde_json::to_value(x).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default() }

fn csv(header: &[&str], rows: impl IntoIterator<Item = Vec<String>>) -> String {
    // A byte-order mark so Excel reads the file as UTF-8.
    let mut out = String::from("\u{feff}");
    out.push_str(&header.join(","));
    out.push_str("\r\n");
    for r in rows {
        out.push_str(&r.join(","));
        out.push_str("\r\n");
    }
    out
}

pub fn history_csv(history: &History, known: &[Known]) -> String {
    let mut keys: Vec<&String> = history.accounts.keys().collect();
    keys.sort();
    let mut rows = vec![];
    for key in keys {
        let (provider, id) = key.split_once(':').unwrap_or(("", key));
        let k = known.iter().find(|k| k.provider.name() == provider && k.id == id);
        let mut windows = history.accounts[key].windows.clone();
        windows.sort_by_key(|w| (w.kind.clone(), w.resets_at));
        for w in windows {
            let start = if w.idle { w.first_obs } else { w.resets_at - Duration::days(7) };
            rows.push(vec![text(provider), text(id), opt(k.and_then(|k| k.email.as_deref())), opt(k.and_then(|k| k.alias.as_deref())),
                text(&w.kind), time(Some(start)), time(Some(w.resets_at)), num(Some(w.peak)), w.hours.to_string(), w.days.to_string(),
                w.idle.to_string(), time(Some(w.first_obs)), time(Some(w.last_obs))]);
        }
    }
    csv(&["provider", "account_id", "email", "alias", "window", "week_start", "resets_at", "peak_percent", "observed_hours", "observed_days", "idle", "first_reading", "last_reading"], rows)
}

pub fn breakdown_csv(rows: &[DailyRow]) -> String {
    csv(&["day", "provider", "account_id", "email", "attribution", "repo", "model", "input", "cache_write", "cached", "output", "total"],
        rows.iter().map(|r| vec![r.day.to_string(), text(r.provider.name()), opt(r.id.as_deref()), opt(r.email.as_deref()), name(r.attribution),
            text(&r.repo), text(&r.model), r.tokens.input.to_string(), r.tokens.cache_write.to_string(), r.tokens.cached.to_string(),
            r.tokens.output.to_string(), r.tokens.total.to_string()]))
}

pub fn subscriptions_csv(report: &Report) -> String {
    csv(&["provider", "account_id", "email", "alias", "plan", "in_use", "verdict", "confidence", "basis", "weeks_counted", "average_peak_percent",
        "median_peak_percent", "limit_weeks", "renewal_looks_unneeded", "this_week_percent", "pace_percent", "runs_out_at", "next_renewal",
        "renewal_source", "renewal_warning", "monthly_cost", "cost_entered", "cancel_after", "ends_at", "plan_weight", "days_tracked"],
        report.accounts.iter().map(|e| {
            let a = &e.assessment;
            vec![text(e.provider.name()), text(&e.id), opt(e.email.as_deref()), opt(e.alias.as_deref()), opt(e.plan.as_deref()), e.active.to_string(),
                name(a.verdict), name(a.confidence), name(a.basis), a.weeks_counted.to_string(), num(a.average_peak), num(a.median_peak),
                a.limit_weeks.to_string(), a.renew_wasteful.to_string(), num(e.this_week.as_ref().map(|w| w.used_percent)),
                num(e.this_week.as_ref().and_then(|w| w.pace_percent)), time(e.this_week.as_ref().and_then(|w| w.runs_out_at)),
                time(e.renewal.as_ref().map(|r| r.at)), e.renewal.as_ref().map(|r| name(r.source)).unwrap_or_default(),
                e.renewal_warning.map(name).unwrap_or_default(), num(e.monthly_cost), e.cost_entered.to_string(),
                e.cancel_after.map(|d| d.to_string()).unwrap_or_default(), time(e.ends_at), num(e.plan_weight), e.days_tracked.to_string()]
        }))
}

#[derive(Debug, Clone, Serialize)]
pub struct Written { pub folder: PathBuf, pub files: Vec<WrittenFile> }

#[derive(Debug, Clone, Serialize)]
pub struct WrittenFile { pub kind: &'static str, pub path: PathBuf, pub rows: usize, pub bytes: u64 }

/// Writes the three files into `folder` (made if missing), named by the local time.
pub fn write(folder: &Path, history: &str, breakdown: &str, subscriptions: &str, stamp: &str) -> Result<Written, String> {
    if !folder.is_absolute() { return Err("export_folder_not_absolute".into()); }
    std::fs::create_dir_all(folder).map_err(|_| "export_folder_unwritable")?;
    let mut files = vec![];
    for (kind, body) in [("weekly_history", history), ("usage_breakdown", breakdown), ("subscriptions", subscriptions)] {
        let path = folder.join(format!("SlopBar-{}-{stamp}.csv", kind.replace('_', "-")));
        std::fs::write(&path, body).map_err(|_| "export_folder_unwritable")?;
        files.push(WrittenFile { kind, rows: body.matches("\r\n").count().saturating_sub(1), bytes: body.len() as u64, path });
    }
    Ok(Written { folder: folder.to_path_buf(), files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Provider;
    use crate::subscriptions::{AccountHistory, WindowRecord};
    use crate::usage_index::{Attribution, Tokens};

    fn at(s: &str) -> DateTime<Utc> { DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc) }

    #[test]
    fn cells_are_quoted_and_never_formulas() {
        assert_eq!(text("plain"), "plain");
        assert_eq!(text("a,b"), "\"a,b\"");
        assert_eq!(text("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(text("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(text("@sum"), "'@sum");
        assert_eq!(text("line\nbreak"), "linebreak");
    }

    #[test]
    fn writes_three_files_with_emails_and_rows() {
        let mut history = History::default();
        history.accounts.insert("Claude:slot-a".into(), AccountHistory { windows: vec![WindowRecord { kind: "weekly".into(), resets_at: at("2026-10-20T00:00:00Z"),
            peak: 87.456, first_obs: at("2026-10-13T01:00:00Z"), last_obs: at("2026-10-19T23:00:00Z"), hours: 150, days: 7, idle: false, local: false }], ..Default::default() });
        let known = vec![Known { provider: Provider::Claude, id: "slot-a".into(), email: Some("a@example.invalid".into()), alias: Some("=cmd".into()) }];
        let h = history_csv(&history, &known);
        let lines: Vec<&str> = h.trim_start_matches('\u{feff}').split("\r\n").collect();
        assert_eq!(lines[0], "provider,account_id,email,alias,window,week_start,resets_at,peak_percent,observed_hours,observed_days,idle,first_reading,last_reading");
        assert_eq!(lines[1], "Claude,slot-a,a@example.invalid,'=cmd,weekly,2026-10-13T00:00:00Z,2026-10-20T00:00:00Z,87.46,150,7,false,2026-10-13T01:00:00Z,2026-10-19T23:00:00Z");
        let rows = vec![DailyRow { day: chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(), provider: Provider::Claude, id: None, email: None,
            attribution: Attribution::Unattributed, repo: "my, repo".into(), model: "claude-opus-5-5".into(), tokens: Tokens { input: 1, cache_write: 2, cached: 3, output: 4, total: 10 } }];
        let b = breakdown_csv(&rows);
        assert!(b.contains("2026-10-20,Claude,,,unattributed,\"my, repo\",claude-opus-5-5,1,2,3,4,10\r\n"), "{b}");
        let report = Report { accounts: vec![], providers: vec![], generated_at: at("2026-10-21T00:00:00Z") };
        let s = subscriptions_csv(&report);
        let dir = crate::test_support::TempDir::new();
        let out = dir.0.join("nested").join("SlopBar Exports");
        let written = write(&out, &h, &b, &s, "2026-10-21-120000").unwrap();
        assert_eq!(written.files.len(), 3);
        assert_eq!(written.files.iter().map(|f| f.rows).collect::<Vec<_>>(), [1, 1, 0]);
        assert!(written.files[0].path.ends_with("SlopBar-weekly-history-2026-10-21-120000.csv"));
        assert_eq!(std::fs::read_to_string(&written.files[1].path).unwrap(), b);
        assert_eq!(write(Path::new("relative"), "", "", "", "x").unwrap_err(), "export_folder_not_absolute");
    }
}
