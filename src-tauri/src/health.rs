//! The self-check behind Settings > Health and the agents' `get_health`: what SlopBar knows about
//! its own state, turned into a list of checks with a reason and the action that fixes each. It
//! is computed from what the app already has (the last snapshot, the refresh's own outcomes, the
//! journal's counters) and never contacts anything.
use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::accounts::Provider;
use crate::model::AccountUsage;

/// A monitoring sign-in whose renewal deadline is this close needs reconnecting soon.
const EXPIRING_DAYS: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    Warning,
    Problem,
}

/// The action that fixes a check, matching one Settings already offers.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Fix {
    ConnectMonitoring { provider: Provider, id: String },
    Refresh,
    RestartApp,
    OpenSettings { page: &'static str },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    /// Stable per kind ("door", "sharing") and per account ("monitoring:claude:<id>").
    pub id: String,
    pub status: Status,
    pub title: String,
    /// One line on why.
    pub reason: String,
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The worst status of any check.
    pub status: Status,
    pub problems: usize,
    pub warnings: usize,
    pub checks: Vec<Check>,
    pub checked_at: DateTime<Utc>,
}

/// What the monitor's saved session says about one account (see `Monitor::health`).
#[derive(Debug, Clone, Default)]
pub struct MonitorState {
    pub reconnect: bool,
    /// The provider's renewal deadline for the refresh token, when it gave one.
    pub refresh_expires_at: Option<DateTime<Utc>>,
}

/// The agent door (see `door::serve`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    Starting,
    Listening,
    /// Opening the pipe failed; the door retries on its own.
    Retrying,
    /// Not a Windows build: there is no door.
    Unsupported,
}

/// Where in-use status is shared, and how the last refresh went with it.
#[derive(Debug, Clone, Default)]
pub struct Sharing {
    pub folder: Option<String>,
    pub remote: Option<String>,
    pub rclone_installed: bool,
    pub last: Option<SharingResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharingResult {
    pub at: DateTime<Utc>,
    pub wrote: bool,
    pub read: bool,
}

/// The diagnostics journal's own counters (see `Recorder::health`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Journal {
    pub started: bool,
    pub write_attempts: u64,
    pub storage_errors: u64,
    pub last_write_ok: Option<bool>,
    pub history_unreadable: bool,
}

/// Windows notifications: whether SlopBar registered its toast identity, and whether Windows
/// lets it (and anything) show toasts. `None` is unknown.
#[derive(Debug, Clone, Copy, Default)]
pub struct Notifications {
    pub registered: Option<bool>,
    pub app_enabled: Option<bool>,
    pub system_enabled: Option<bool>,
    /// Any of SlopBar's own notification kinds is on.
    pub any_wanted: bool,
}

pub struct Inputs<'a> {
    pub accounts: &'a [AccountUsage],
    /// Keyed like `monitoring::key`.
    pub monitoring: &'a HashMap<String, MonitorState>,
    pub refresh_minutes: u32,
    /// The last refresh that read at least one account (or had none to read).
    pub last_success: Option<DateTime<Utc>>,
    pub door: Door,
    pub sharing: &'a Sharing,
    pub journal: Journal,
    pub notifications: Notifications,
    /// Settings: Connect monitoring automatically.
    pub auto_connect: bool,
    /// Saved sign-ins still stored as plaintext (see `accounts::migrate_vault`).
    pub vault_plaintext: usize,
    /// SlopBar's processes in this Windows session, when they could be listed.
    pub processes: Option<crate::instance::Processes>,
    /// Space SlopBar takes on disk, when it could be measured.
    pub disk: Option<Disk>,
}

/// What SlopBar keeps on disk: its app data (diagnostics exports and captures included) and the
/// rollback copies local installs leave (`scripts/install-local-build.ps1`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Disk {
    pub app_data: u64,
    pub diagnostics: u64,
    pub rollback: u64,
    pub rollback_folders: usize,
}

impl Disk {
    /// Measures the app data folder and the rollback folder.
    pub fn measure(app_data: &std::path::Path, rollback: &std::path::Path) -> Self {
        let folders = std::fs::read_dir(rollback).map(|d| d.flatten().filter(|e| e.path().is_dir()).count()).unwrap_or(0);
        Disk { app_data: size(app_data), diagnostics: size(&app_data.join("diagnostics")), rollback: size(rollback), rollback_folders: folders }
    }
}

/// Bytes in the files under `path` (symbolic links are not followed).
pub fn size(path: &std::path::Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    std::fs::read_dir(path).map(|d| d.flatten().map(|e| size(&e.path())).sum()).unwrap_or(0)
}

/// "1.2 GB", "683 MB", "4 KB".
pub fn bytes(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let n = n as f64;
    if n >= 1024.0 * MB { format!("{:.1} GB", n / (1024.0 * MB)) }
    else if n >= MB { format!("{:.0} MB", n / MB) }
    else { format!("{:.0} KB", (n / 1024.0).ceil()) }
}

fn name(a: &AccountUsage) -> String {
    a.alias.clone().or_else(|| a.email.clone()).unwrap_or_else(|| format!("{} account", label(a.provider)))
}

fn label(p: Provider) -> &'static str {
    match p {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

fn ago(now: DateTime<Utc>, at: DateTime<Utc>) -> String {
    let minutes = (now - at).num_minutes().max(0);
    match minutes {
        0 => "less than a minute ago".into(),
        1 => "1 minute ago".into(),
        m if m < 120 => format!("{m} minutes ago"),
        m if m < 48 * 60 => format!("{} hours ago", m / 60),
        m => format!("{} days ago", m / (24 * 60)),
    }
}

fn check(id: impl Into<String>, status: Status, title: impl Into<String>, reason: impl Into<String>, fix: Option<Fix>) -> Check {
    Check { id: id.into(), status, title: title.into(), reason: reason.into(), fix }
}

/// Every check, worst first within the order Settings lists them.
pub fn checks(inputs: &Inputs, now: DateTime<Utc>) -> Report {
    let interval = Duration::minutes(inputs.refresh_minutes.max(1) as i64);
    // Two refresh intervals, plus slack for a slow poll.
    let max_age = interval * 2 + Duration::seconds(60);
    let mut checks = vec![];

    // Monitoring sign-ins, one row per account that needs something; a single row otherwise.
    let mut monitoring = vec![];
    for a in inputs.accounts {
        // Monitor sessions are keyed like `monitoring::key`; check ids use the lowercase provider key.
        let key = format!("{}:{}", a.provider.name(), a.id);
        let id = format!("monitoring:{}:{}", a.provider.key(), a.id);
        let who = name(a);
        let connect = Some(Fix::ConnectMonitoring { provider: a.provider, id: a.id.clone() });
        if !a.monitor_connected {
            let how = if inputs.auto_connect {
                "Connect monitoring automatically is on (Settings > Data): SlopBar starts its sign-in once it has been in use for 3 minutes, or connect it now."
            } else {
                "Connect monitoring automatically is off (Settings > Data): connect it now, or turn that on to have SlopBar start the sign-in when it is in use."
            };
            monitoring.push(check(id, Status::Warning, format!("{who}: monitoring not connected"),
                format!("Its usage is only read while it is the account in use. {how}"), connect));
            continue;
        }
        let state = inputs.monitoring.get(&key).cloned().unwrap_or_default();
        let error = a.monitor_error.as_deref();
        if state.reconnect || matches!(error, Some("monitor_reconnect" | "monitor_uncertain" | "monitor_wrong_account" | "monitor_setup")) {
            monitoring.push(check(id, Status::Problem, format!("{who}: monitoring sign-in needs reconnecting"),
                "The provider no longer accepts its monitoring sign-in. Reconnecting needs no account use.", connect));
        } else if let Some(at) = state.refresh_expires_at.filter(|t| *t > now && *t <= now + Duration::days(EXPIRING_DAYS)) {
            let hours = (at - now).num_hours();
            let when = if hours < 48 { format!("within {} hours", hours.max(1)) } else { format!("in {} days", hours / 24) };
            monitoring.push(check(id, Status::Warning, format!("{who}: monitoring sign-in expires soon"),
                format!("The provider says its monitoring sign-in expires {when}; reconnect before then to avoid a gap."), connect));
        } else if error == Some("monitor_storage") {
            monitoring.push(check(id, Status::Problem, format!("{who}: monitoring sign-in unreadable"),
                "The saved monitoring sign-in could not be read or saved. Check disk space and file access, then reconnect.", connect));
        } else if let Some(e) = error {
            let why = if e == "monitor_network" { "The monitoring service could not be reached; it is retried at each refresh.".to_string() }
                else if e.starts_with("rate_limited") { "The provider is rate-limiting monitoring; it is retried later.".to_string() }
                else { format!("Monitoring failed ({}); it is retried at each refresh.", e.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(32).collect::<String>()) };
            monitoring.push(check(id, Status::Warning, format!("{who}: monitoring failing"), why, Some(Fix::Refresh)));
        }
    }
    if monitoring.is_empty() {
        let reason = if inputs.accounts.is_empty() { "No accounts yet.".to_string() } else { format!("All {} accounts have working monitoring sign-ins.", inputs.accounts.len()) };
        checks.push(check("monitoring", Status::Ok, "Monitoring sign-ins", reason, None));
    }
    checks.extend(monitoring);

    // The CLI's own saved login, which SlopBar switches but never renews or signs in again.
    for a in inputs.accounts.iter().filter(|a| a.cli_rejected) {
        let cli = match a.provider { Provider::Claude => "claude /login", Provider::Codex => "codex login" };
        let how = if a.active { format!("Run `{cli}` to sign the CLI in again.") }
            else { format!("Switch to it and run `{cli}`, or remove it if it is no longer used.") };
        checks.push(check(format!("cli:{}:{}", a.provider.key(), a.id), Status::Problem, format!("{}: CLI sign-in rejected", name(a)),
            format!("The provider no longer accepts the CLI's saved sign-in for this account; the CLI manages that sign-in. {how}"),
            Some(Fix::OpenSettings { page: "accounts" })));
    }
    if inputs.vault_plaintext > 0 {
        checks.push(check("vault", Status::Warning, "Saved sign-ins",
            format!("{} saved sign-in(s) could not be encrypted yet and are still stored as plaintext; SlopBar retries at each start. Check disk space and file access.", inputs.vault_plaintext),
            Some(Fix::RestartApp)));
    }

    // Readings older than two refresh intervals. An unmonitored account not in use is already
    // reported above: its reading cannot be fresh until monitoring is connected.
    let mut stale = vec![];
    for a in inputs.accounts.iter().filter(|a| a.monitor_connected || a.active) {
        let old = a.fetched_at.map_or(true, |t| now - t > max_age);
        if !(a.stale || old) || inputs.last_success.is_none() {
            continue;
        }
        let who = name(a);
        let reason = match a.fetched_at {
            Some(t) => format!("Last confirmed {}, more than two refresh intervals ago.", ago(now, t)),
            None => "No usage has been read for it yet.".into(),
        };
        stale.push(check(format!("freshness:{}:{}", a.provider.key(), a.id), Status::Warning, format!("{who}: usage is out of date"), reason, Some(Fix::Refresh)));
    }
    if stale.is_empty() {
        checks.push(check("freshness", Status::Ok, "Usage readings", "Every monitored account's reading is current.", None));
    }
    checks.extend(stale);

    checks.push(match inputs.last_success {
        None => check("refresh", Status::Warning, "Last refresh", "No refresh has read any account yet.", Some(Fix::Refresh)),
        Some(t) if now - t > max_age => check("refresh", Status::Problem, "Last refresh",
            format!("The last refresh that read usage was {}; refreshes run every {} minutes.", ago(now, t), interval.num_minutes()), Some(Fix::Refresh)),
        Some(t) => check("refresh", Status::Ok, "Last refresh", format!("Usage was read {}.", ago(now, t)), None),
    });

    if let Some(p) = inputs.processes {
        checks.push(if p.instances > 1 {
            check("instances", Status::Problem, "SlopBar is running more than once",
                format!("{} copies are running and each renews the same monitoring sign-ins, which can get them rejected. Restarting leaves one.", p.instances),
                Some(Fix::RestartApp))
        } else {
            check("instances", Status::Ok, "Running once", "One copy of SlopBar is running.", None)
        });
        // Information only: each agent session starts its own server, which ends with the session.
        checks.push(check("agent_servers", Status::Ok, "Agent servers", match p.mcp_servers {
            0 => "No agent is connected (slopbar-mcp).".to_string(),
            1 => "1 agent server (slopbar-mcp) is running.".to_string(),
            n => format!("{n} agent servers (slopbar-mcp) are running, one per agent session."),
        }, None));
    }

    // Information only: exports, captures and rollbacks are pruned on their own.
    if let Some(d) = inputs.disk {
        let rollback = match d.rollback_folders {
            0 => "no install rollbacks".to_string(),
            1 => format!("1 install rollback, {}", bytes(d.rollback)),
            n => format!("{n} install rollbacks, {}", bytes(d.rollback)),
        };
        checks.push(check("disk", Status::Ok, "Disk use",
            format!("App data {} (diagnostics {}); {rollback}.", bytes(d.app_data), bytes(d.diagnostics)), None));
    }

    match inputs.door {
        Door::Unsupported => {}
        Door::Listening => checks.push(check("door", Status::Ok, "Agent door", "Listening for agents (slopbar-mcp).", None)),
        Door::Starting => checks.push(check("door", Status::Warning, "Agent door", "Still opening; agents can connect shortly.", None)),
        Door::Retrying => checks.push(check("door", Status::Problem, "Agent door",
            "Agents cannot reach SlopBar: the door could not be opened (another copy may still be exiting). Restarting reopens it.", Some(Fix::RestartApp))),
    }

    let sharing = inputs.sharing;
    let data = Some(Fix::OpenSettings { page: "data" });
    if let Some(remote) = &sharing.remote {
        checks.push(if sharing.rclone_installed {
            check("rclone", Status::Ok, "rclone", format!("Installed, for the remote {remote}."), None)
        } else {
            check("rclone", Status::Problem, "rclone", format!("Sharing uses the rclone remote {remote}, but rclone is not installed, so nothing is shared."), data.clone())
        });
    }
    let place = match (&sharing.remote, &sharing.folder) {
        (Some(r), _) => Some(format!("the rclone remote {r}")),
        (None, Some(f)) => Some(format!("the folder {f}")),
        (None, None) => None,
    };
    checks.push(match (place, sharing.last) {
        (None, _) => check("sharing", Status::Ok, "In-use sharing", "Off: in-use status is not shared with other computers.", None),
        (Some(_), _) if sharing.remote.is_some() && !sharing.rclone_installed => check("sharing", Status::Problem, "In-use sharing",
            "Not running: rclone is missing.", data),
        (Some(p), None) => check("sharing", Status::Ok, "In-use sharing", format!("Shares through {p}; checked at the next refresh."), None),
        (Some(p), Some(r)) => match (r.wrote, r.read) {
            (true, true) => check("sharing", Status::Ok, "In-use sharing", format!("Shared through {p} {}.", ago(now, r.at)), None),
            (false, true) => check("sharing", Status::Problem, "In-use sharing", format!("Could not write to {p}, so other computers do not see this one."), data),
            (true, false) => check("sharing", Status::Problem, "In-use sharing", format!("Could not read {p}, so this computer does not see the others."), data),
            (false, false) => check("sharing", Status::Problem, "In-use sharing", format!("Could not write to or read {p}."), data),
        },
    });

    let j = inputs.journal;
    checks.push(if !j.started {
        check("journal", Status::Warning, "Diagnostics journal", "Not started yet.", None)
    } else if j.last_write_ok == Some(false) {
        check("journal", Status::Problem, "Diagnostics journal",
            format!("Could not be saved ({} of {} writes failed). Check disk space and file access; monitoring is unaffected.", j.storage_errors, j.write_attempts), None)
    } else if j.history_unreadable {
        check("journal", Status::Warning, "Diagnostics journal", "Earlier history could not be read and was started afresh.", None)
    } else if j.write_attempts == 0 {
        check("journal", Status::Ok, "Diagnostics journal", "Writable; nothing needed saving yet.", None)
    } else {
        check("journal", Status::Ok, "Diagnostics journal", "Saved normally.", None)
    });

    let n = inputs.notifications;
    let open = Some(Fix::OpenSettings { page: "notifications" });
    checks.push(if !n.any_wanted {
        check("notifications", Status::Ok, "Notifications", "Every SlopBar notification is turned off.", open)
    } else if n.system_enabled == Some(false) {
        check("notifications", Status::Warning, "Notifications", "Windows notifications are turned off for every app (Windows Settings > System > Notifications).", None)
    } else if n.app_enabled == Some(false) {
        check("notifications", Status::Warning, "Notifications", "Windows notifications are turned off for SlopBar (Windows Settings > System > Notifications).", None)
    } else if n.registered == Some(false) {
        check("notifications", Status::Warning, "Notifications", "SlopBar could not register its notification name and icon; restarting retries.", Some(Fix::RestartApp))
    } else {
        check("notifications", Status::Ok, "Notifications", "Registered with Windows.", None)
    });

    let problems = checks.iter().filter(|c| c.status == Status::Problem).count();
    let warnings = checks.iter().filter(|c| c.status == Status::Warning).count();
    let status = checks.iter().map(|c| c.status).max().unwrap_or(Status::Ok);
    Report { status, problems, warnings, checks, checked_at: now }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 21, 12, 0, 0).unwrap()
    }

    fn account(id: &str, monitored: bool, fetched_minutes_ago: Option<i64>) -> AccountUsage {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "claude", "email": format!("{id}@example.invalid"), "alias": null, "plan": "max", "org": null,
            "active": false, "sources": [], "windows": [], "credits": null, "resets": [], "error": null,
            "stale": false, "fetched_at": fetched_minutes_ago.map(|m| now() - Duration::minutes(m)), "monitor_connected": monitored,
        })).unwrap()
    }

    struct Case {
        accounts: Vec<AccountUsage>,
        monitoring: HashMap<String, MonitorState>,
        last_success: Option<DateTime<Utc>>,
        door: Door,
        sharing: Sharing,
        journal: Journal,
        notifications: Notifications,
        auto_connect: bool,
        vault_plaintext: usize,
        processes: Option<crate::instance::Processes>,
        disk: Option<Disk>,
    }

    impl Case {
        fn healthy() -> Self {
            Case {
                accounts: vec![account("a", true, Some(1)), account("b", true, Some(2))],
                monitoring: HashMap::new(),
                last_success: Some(now() - Duration::minutes(1)),
                door: Door::Listening,
                sharing: Sharing::default(),
                journal: Journal { started: true, write_attempts: 4, storage_errors: 0, last_write_ok: Some(true), history_unreadable: false },
                notifications: Notifications { registered: Some(true), app_enabled: None, system_enabled: None, any_wanted: true },
                auto_connect: true,
                vault_plaintext: 0,
                processes: Some(crate::instance::Processes { instances: 1, mcp_servers: 3 }),
                disk: None,
            }
        }

        fn run(&self) -> Report {
            checks(&Inputs { accounts: &self.accounts, monitoring: &self.monitoring, refresh_minutes: 5, last_success: self.last_success,
                door: self.door, sharing: &self.sharing, journal: self.journal, notifications: self.notifications, auto_connect: self.auto_connect,
                vault_plaintext: self.vault_plaintext, processes: self.processes, disk: self.disk }, now())
        }
    }

    fn find<'a>(r: &'a Report, id: &str) -> &'a Check {
        r.checks.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no {id} in {:?}", r.checks.iter().map(|c| &c.id).collect::<Vec<_>>()))
    }

    #[test]
    fn a_healthy_machine_is_all_ok() {
        let r = Case::healthy().run();
        assert_eq!((r.status, r.problems, r.warnings), (Status::Ok, 0, 0), "{:#?}", r.checks);
        let ids: Vec<_> = r.checks.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["monitoring", "freshness", "refresh", "instances", "agent_servers", "door", "sharing", "journal", "notifications"]);
    }

    #[test]
    fn disk_use_is_information_with_app_data_and_rollback_sizes() {
        let mut case = Case::healthy();
        case.disk = Some(Disk { app_data: 24 * 1024 * 1024, diagnostics: 12 * 1024 * 1024, rollback: 61 * 1024 * 1024, rollback_folders: 3 });
        let r = case.run();
        let d = find(&r, "disk");
        assert_eq!((d.status, d.fix.clone()), (Status::Ok, None));
        assert_eq!(d.reason, "App data 24 MB (diagnostics 12 MB); 3 install rollbacks, 61 MB.");
        case.disk = Some(Disk { app_data: 3000, ..Disk::default() });
        assert_eq!(find(&case.run(), "disk").reason, "App data 3 KB (diagnostics 0 KB); no install rollbacks.");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
    }

    #[test]
    fn disk_use_is_measured_from_the_files() {
        let dir = crate::test_support::TempDir::new();
        let data = dir.0.join("data");
        std::fs::create_dir_all(data.join("diagnostics/exports")).unwrap();
        std::fs::write(data.join("settings.json"), vec![0u8; 100]).unwrap();
        std::fs::write(data.join("diagnostics/exports/a.json"), vec![0u8; 400]).unwrap();
        let rollback = dir.0.join("rollback");
        for name in ["20261001-100000", "20261002-100000"] {
            std::fs::create_dir_all(rollback.join(name)).unwrap();
            std::fs::write(rollback.join(name).join("slopbar.exe"), vec![0u8; 1000]).unwrap();
        }
        assert_eq!(Disk::measure(&data, &rollback), Disk { app_data: 500, diagnostics: 400, rollback: 2000, rollback_folders: 2 });
        assert_eq!(Disk::measure(&dir.0.join("absent"), &dir.0.join("absent")), Disk::default());
    }

    #[test]
    fn a_second_copy_is_a_problem_and_agent_servers_are_only_information() {
        let mut case = Case::healthy();
        case.processes = Some(crate::instance::Processes { instances: 2, mcp_servers: 25 });
        let r = case.run();
        let c = find(&r, "instances");
        assert_eq!((c.status, c.fix.clone(), c.title.as_str()), (Status::Problem, Some(Fix::RestartApp), "SlopBar is running more than once"));
        let servers = find(&r, "agent_servers");
        assert_eq!((servers.status, servers.fix.clone()), (Status::Ok, None));
        assert!(servers.reason.starts_with("25 agent servers"));
        // Unknown (the list could not be read): no claim either way.
        case.processes = None;
        let r = case.run();
        assert!(!r.checks.iter().any(|c| c.id == "instances" || c.id == "agent_servers"));
    }

    #[test]
    fn monitoring_that_needs_reconnecting_is_a_problem_with_its_fix() {
        let mut case = Case::healthy();
        case.accounts[0].monitor_error = Some("monitor_reconnect".into());
        case.monitoring.insert("Claude:b".into(), MonitorState { reconnect: true, refresh_expires_at: None });
        let r = case.run();
        for id in ["a", "b"] {
            let c = find(&r, &format!("monitoring:claude:{id}"));
            assert_eq!(c.status, Status::Problem);
            assert_eq!(c.fix, Some(Fix::ConnectMonitoring { provider: Provider::Claude, id: id.into() }));
            assert!(c.title.contains(&format!("{id}@example.invalid")));
        }
        assert!(r.checks.iter().all(|c| c.id != "monitoring"), "the all-ok row is replaced");
        assert_eq!((r.status, r.problems), (Status::Problem, 2));
    }

    #[test]
    fn a_rejected_cli_login_and_plaintext_sign_ins_are_reported() {
        let mut case = Case::healthy();
        case.accounts[1].cli_rejected = true;
        case.accounts[1].active = true;
        case.vault_plaintext = 2;
        let r = case.run();
        let c = find(&r, "cli:claude:b");
        assert_eq!((c.status, c.fix.clone()), (Status::Problem, Some(Fix::OpenSettings { page: "accounts" })));
        assert!(c.reason.contains("claude /login") && c.reason.contains("the CLI manages"), "{}", c.reason);
        assert!(r.checks.iter().all(|c| c.id != "cli:claude:a"));
        let v = find(&r, "vault");
        assert_eq!((v.status, v.fix.clone()), (Status::Warning, Some(Fix::RestartApp)));
        assert!(v.reason.starts_with("2 "));
        assert_eq!((r.problems, r.warnings), (1, 1));
    }

    #[test]
    fn an_unmonitored_account_says_whether_it_connects_automatically() {
        let mut case = Case::healthy();
        case.accounts[0].monitor_connected = false;
        let r = case.run();
        let c = find(&r, "monitoring:claude:a");
        assert_eq!(c.status, Status::Warning);
        assert_eq!(c.fix, Some(Fix::ConnectMonitoring { provider: Provider::Claude, id: "a".into() }));
        assert!(c.reason.contains("Connect monitoring automatically is on"), "{}", c.reason);
        case.auto_connect = false;
        let r = case.run();
        let c = find(&r, "monitoring:claude:a");
        assert!(c.reason.contains("Connect monitoring automatically is off"), "{}", c.reason);
    }

    #[test]
    fn a_monitoring_sign_in_expiring_within_three_days_is_a_warning() {
        let mut case = Case::healthy();
        case.monitoring.insert("Claude:a".into(), MonitorState { reconnect: false, refresh_expires_at: Some(now() + Duration::hours(30)) });
        case.monitoring.insert("Claude:b".into(), MonitorState { reconnect: false, refresh_expires_at: Some(now() + Duration::days(20)) });
        let r = case.run();
        let c = find(&r, "monitoring:claude:a");
        assert_eq!(c.status, Status::Warning);
        assert!(c.reason.contains("within 30 hours"), "{}", c.reason);
        assert!(r.checks.iter().all(|c| c.id != "monitoring:claude:b"), "far-off deadlines are fine");
        // A passed deadline is only a renewal hint: renewing at the next refresh clears it.
        let mut passed = Case::healthy();
        passed.monitoring.insert("Claude:a".into(), MonitorState { reconnect: false, refresh_expires_at: Some(now() - Duration::hours(1)) });
        assert_eq!(passed.run().status, Status::Ok);
    }

    #[test]
    fn transient_monitoring_failures_warn_and_retry() {
        let mut case = Case::healthy();
        case.accounts[0].monitor_error = Some("monitor_network".into());
        case.accounts[1].monitor_error = Some("weird\nprovider text that is long".into());
        let r = case.run();
        let a = find(&r, "monitoring:claude:a");
        assert_eq!((a.status, a.fix.clone()), (Status::Warning, Some(Fix::Refresh)));
        let b = find(&r, "monitoring:claude:b");
        assert!(!b.reason.contains('\n') && !b.reason.contains("provider text that is long"), "raw errors are not echoed: {}", b.reason);
        case.accounts[0].monitor_error = Some("monitor_storage".into());
        assert_eq!(find(&case.run(), "monitoring:claude:a").status, Status::Problem);
    }

    #[test]
    fn accounts_without_monitoring_are_warnings_to_connect() {
        let mut case = Case::healthy();
        case.accounts.push(account("c", false, Some(600)));
        let r = case.run();
        let c = find(&r, "monitoring:claude:c");
        assert_eq!((c.status, c.fix.clone()), (Status::Warning, Some(Fix::ConnectMonitoring { provider: Provider::Claude, id: "c".into() })));
        assert!(r.checks.iter().all(|c| c.id != "freshness:claude:c"), "its old reading is explained by the monitoring row");
        assert_eq!((r.status, r.warnings), (Status::Warning, 1));
    }

    #[test]
    fn readings_older_than_two_intervals_are_stale() {
        let mut case = Case::healthy();
        case.accounts[0].fetched_at = Some(now() - Duration::minutes(11) - Duration::seconds(30));
        case.accounts[1].stale = true;
        let mut active = account("in-use", false, Some(30));
        active.active = true;
        case.accounts.push(active);
        let r = case.run();
        assert_eq!(find(&r, "freshness:claude:a").status, Status::Warning);
        assert!(find(&r, "freshness:claude:a").reason.contains("11 minutes ago"));
        assert_eq!(find(&r, "freshness:claude:b").fix, Some(Fix::Refresh));
        assert_eq!(find(&r, "freshness:claude:in-use").status, Status::Warning, "the account in use is read through its CLI login");
        // Ten minutes and a half is within two five-minute intervals plus slack.
        case.accounts = vec![account("a", true, Some(10))];
        assert_eq!(find(&case.run(), "freshness").status, Status::Ok);
    }

    #[test]
    fn last_successful_refresh_age() {
        let mut case = Case::healthy();
        case.last_success = None;
        let r = case.run();
        assert_eq!(find(&r, "refresh").status, Status::Warning);
        assert!(r.checks.iter().all(|c| !c.id.starts_with("freshness:")), "no staleness before the first refresh");
        case.last_success = Some(now() - Duration::minutes(45));
        let r = case.run();
        let c = find(&r, "refresh");
        assert_eq!((c.status, c.fix.clone()), (Status::Problem, Some(Fix::Refresh)));
        assert!(c.reason.contains("45 minutes ago"), "{}", c.reason);
    }

    #[test]
    fn the_agent_door() {
        let mut case = Case::healthy();
        case.door = Door::Retrying;
        let c = find(&case.run(), "door").clone();
        assert_eq!((c.status, c.fix), (Status::Problem, Some(Fix::RestartApp)));
        case.door = Door::Starting;
        assert_eq!(find(&case.run(), "door").status, Status::Warning);
        case.door = Door::Unsupported;
        assert!(case.run().checks.iter().all(|c| c.id != "door"));
    }

    #[test]
    fn sharing_that_cannot_write_or_read_points_to_data_settings() {
        let mut case = Case::healthy();
        case.sharing = Sharing { folder: Some("D:\\Sync".into()), remote: None, rclone_installed: false, last: None };
        assert_eq!(find(&case.run(), "sharing").status, Status::Ok, "not yet tried");
        assert!(case.run().checks.iter().all(|c| c.id != "rclone"), "rclone only matters for a remote");
        for (wrote, read, status) in [(true, true, Status::Ok), (false, true, Status::Problem), (true, false, Status::Problem), (false, false, Status::Problem)] {
            case.sharing.last = Some(SharingResult { at: now() - Duration::minutes(2), wrote, read });
            let r = case.run();
            let c = find(&r, "sharing");
            assert_eq!(c.status, status, "wrote {wrote} read {read}");
            if status == Status::Problem {
                assert_eq!(c.fix, Some(Fix::OpenSettings { page: "data" }));
            }
        }
    }

    #[test]
    fn a_chosen_remote_without_rclone_is_a_problem() {
        let mut case = Case::healthy();
        case.sharing = Sharing { folder: Some("D:\\Sync".into()), remote: Some("gdrive".into()), rclone_installed: false, last: None };
        let r = case.run();
        assert_eq!(find(&r, "rclone").status, Status::Problem);
        assert_eq!(find(&r, "rclone").fix, Some(Fix::OpenSettings { page: "data" }));
        assert_eq!(find(&r, "sharing").status, Status::Problem);
        case.sharing.rclone_installed = true;
        assert_eq!(find(&case.run(), "rclone").status, Status::Ok);
    }

    #[test]
    fn the_diagnostics_journal() {
        let mut case = Case::healthy();
        case.journal = Journal { started: true, write_attempts: 3, storage_errors: 3, last_write_ok: Some(false), history_unreadable: false };
        let c = find(&case.run(), "journal").clone();
        assert_eq!(c.status, Status::Problem);
        assert!(c.reason.contains("3 of 3"));
        case.journal.last_write_ok = Some(true);
        assert_eq!(find(&case.run(), "journal").status, Status::Ok, "a later write succeeded");
        case.journal.history_unreadable = true;
        assert_eq!(find(&case.run(), "journal").status, Status::Warning);
        case.journal = Journal { started: true, ..Journal::default() };
        assert_eq!(find(&case.run(), "journal").status, Status::Ok);
    }

    #[test]
    fn notification_registration_and_windows_settings() {
        let mut case = Case::healthy();
        case.notifications.registered = Some(false);
        let c = find(&case.run(), "notifications").clone();
        assert_eq!((c.status, c.fix), (Status::Warning, Some(Fix::RestartApp)));
        case.notifications.app_enabled = Some(false);
        assert!(find(&case.run(), "notifications").reason.contains("for SlopBar"));
        case.notifications.system_enabled = Some(false);
        assert!(find(&case.run(), "notifications").reason.contains("every app"));
        case.notifications.any_wanted = false;
        assert_eq!(find(&case.run(), "notifications").status, Status::Ok, "nothing to show, nothing wrong");
    }

    #[test]
    fn the_report_serializes_for_agents() {
        let mut case = Case::healthy();
        case.door = Door::Retrying;
        let v = serde_json::to_value(case.run()).unwrap();
        assert_eq!(v["status"], "problem");
        assert_eq!(v["problems"], 1);
        let door = v["checks"].as_array().unwrap().iter().find(|c| c["id"] == "door").unwrap();
        assert_eq!(door["fix"], serde_json::json!({ "action": "restart_app" }));
    }
}
