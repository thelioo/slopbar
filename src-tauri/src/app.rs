use crate::{accounts, active_log, auto_monitor, backup, balancer, bulk, burn, calendar, codex_local, csv_export, desktop, diagnostics, digest, dock, door, headroom, health, inspect, leases, ledger, migrate, model, monitoring, notify, pairing, panel, phone, pins, plan_alerts, presence, providers, reset_credits, roster, settings, signin, sources, subscriptions, unexpected, updates, usage_index, wind_down};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;

use accounts::Provider;
use model::Snapshot;
use settings::Settings;

pub(crate) const SETTINGS_LABEL: &str = "settings";
const MENU_IDS: [&str; 3] = ["refresh", "settings", "quit"];
/// The menus' checkable item that turns the compact flyout on and off.
pub(crate) const MENU_COMPACT: &str = "compact_flyout";

pub struct AppState {
    monitor: monitoring::Monitor,
    diagnostics: std::sync::Arc<diagnostics::Recorder>,
    refresh_lock: tokio::sync::Mutex<()>,
    /// Held from reading settings to storing them, so the window and agents never lose a change.
    settings_lock: tokio::sync::Mutex<()>,
    account_operations: monitoring::Operations,
    pub(crate) snapshot: Mutex<Snapshot>,
    pub(crate) settings: Mutex<Settings>,
    /// The widget's right-click menu and its items (relabeled when the language changes).
    pub(crate) menu: Mutex<Option<Menu<Wry>>>,
    pub(crate) menu_items: Mutex<Vec<MenuItem<Wry>>>,
    /// Physical center of the widget when it was last clicked; the flyout opens above it.
    pub(crate) panel_anchor: Mutex<Option<(f64, f64)>>,
    pub(crate) panel_height: Mutex<f64>,
    pub(crate) panel_hidden_at: Mutex<Option<Instant>>,
    /// Content width of the taskbar widget, in CSS px.
    pub(crate) dock_width: Mutex<f64>,
    /// Last known usage per account, for accounts whose saved token can't be used right now.
    pub(crate) usage_cache: Mutex<providers::Cache>,
    /// Every account's completed weekly windows, for the Subscriptions page.
    pub(crate) usage_history: Mutex<subscriptions::History>,
    /// The Codex CLI's own history on this PC, read in the background after each refresh.
    pub(crate) codex_local: Mutex<Option<std::sync::Arc<codex_local::Local>>>,
    /// Which account each CLI had signed in when (see `active_log`), and when it was last saved.
    pub(crate) active_log: Mutex<active_log::Log>,
    active_saved: Mutex<Option<Instant>>,
    /// Tokens by account, repository and model from the CLIs' files here, read after each refresh.
    pub(crate) usage_index: Mutex<Option<std::sync::Arc<usage_index::Index>>>,
    /// When each provider was last switched automatically (the balancer's cooldown).
    pub(crate) last_switch: Mutex<HashMap<Provider, Instant>>,
    /// A graceful switch waiting for Claude Code sessions to wrap up (see `wind_down`).
    pub(crate) wind_down: Mutex<wind_down::Tracker>,
    /// The past few hours of readings, for the agents' pace forecasts.
    pub(crate) recent: Mutex<headroom::Recent>,
    /// Sources found by the last refresh, where switches are written.
    pub(crate) sources: Mutex<Vec<sources::Source>>,
    /// Recent readings of unclaimed accounts, to notice use outside this PC.
    presence_tracks: Mutex<presence::Tracks>,
    /// The account roster's keys and last write (see `roster`), held while it syncs.
    roster: std::sync::Arc<Mutex<roster::Syncer>>,
    /// What the roster's last sync found, for Settings, Health and sign-ins.
    roster_seen: Mutex<roster::Seen>,
    /// "Sign in all accounts" as the roster carries it to the other computers (see `roster::Run`).
    roster_run: Mutex<Option<roster::Run>>,
    /// Use seen where none of the user's computers is, for a week (see `unexpected`).
    unexpected_log: Mutex<unexpected::Log>,
    /// Plan notifications already shown (renewals, running out), so each is shown once.
    plan_alerts: Mutex<plan_alerts::Sent>,
    /// Runaway agents seen in the past day (see `burn`), and notifications not yet shown.
    burn_log: Mutex<burn::Log>,
    burn_due: Mutex<Vec<burn::BurnAlert>>,
    /// What the plans have cost month by month (see `ledger`).
    ledger: Mutex<ledger::Ledger>,
    /// Connect monitoring automatically: how long accounts have been in use, and backoffs.
    auto_monitor: Mutex<auto_monitor::Tracker>,
    /// What refreshes and startup learned for Settings > Health.
    health: Mutex<HealthSeen>,
    /// Where each pinned account's project profile stood after the last sync (see `pins`).
    profile_states: Mutex<HashMap<(Provider, String), accounts::ProfileState>>,
    /// The queued "Connect monitoring for all" while it runs: the accounts left, done first.
    bulk_connect: Mutex<Option<BulkConnect>>,
    /// Stops the running one (Settings' Stop).
    bulk_stop: Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// Woken at every change to `bulk_connect` (see `wait_for_sign_in_change`).
    bulk_changed: tokio::sync::Notify,
    bulk_revision: std::sync::atomic::AtomicU64,
    /// Why the switcher's shortcut could not be registered (taken by another app, say).
    hotkey_error: Mutex<Option<String>>,
    /// The same for the compact toggle's shortcut.
    compact_hotkey_error: Mutex<Option<String>>,
    /// The widget menu's compact flyout item, checked while the setting is on.
    compact_item: Mutex<Option<CheckMenuItem<Wry>>>,
    /// The page Settings opens on when it is being created (it is destroyed when closed).
    settings_page: Mutex<Option<String>>,
    /// Accounts reserved for jobs (see `leases`); in memory only.
    pub(crate) leases: Mutex<leases::Leases>,
}

pub(crate) const SWITCHER_LABEL: &str = "switcher";

/// Progress of "Connect monitoring for all" and "Sign in all accounts", sent to Settings as
/// `bulk-monitoring`.
#[derive(Clone, Serialize)]
struct BulkConnect {
    total: usize,
    done: usize,
    /// The account whose sign-in is open now: sign in as this one.
    current: Option<bulk::Target>,
    /// What the open sign-in is for (None: a monitoring sign-in of "Connect monitoring for all").
    kind: Option<bulk::Kind>,
    /// "Sign in all accounts": how many emails, and which one (1-based) is signing in now.
    emails: usize,
    email: usize,
    finished: bool,
    error: Option<String>,
    /// Every sign-in of the run and how it went, kept once it ends, so an agent elsewhere can tell
    /// the person at the desktop exactly what happened.
    steps: Vec<StepRecord>,
    /// What Retry runs again: "Sign in all accounts" (`by_email`, with or without `monitoring`),
    /// else "Connect monitoring for all".
    by_email: bool,
    monitoring: bool,
    /// The open sign-in has waited long enough that the person was asked whether to retry it.
    still_waiting: bool,
    /// Bumped at every change, across runs (see `wait_for_sign_in_change`).
    revision: u64,
}

/// One sign-in of a run.
#[derive(Clone, Serialize)]
struct StepRecord {
    #[serde(flatten)]
    target: bulk::Target,
    kind: bulk::Kind,
    state: StepState,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// When it started, when its page opened (in an email's profile, the only place that tells)
    /// and when it ended, so a run's waits can be measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    page_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ended_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StepState {
    /// Not reached: still to come, or the run stopped before it.
    Pending,
    Running,
    Done,
    /// Already done (or the account removed) by the time the run reached it.
    Skipped,
    Failed,
}

/// Outcomes `health::checks` needs that nothing else keeps.
#[derive(Clone, Default)]
struct HealthSeen {
    last_success: Option<chrono::DateTime<chrono::Utc>>,
    sharing: Option<health::SharingResult>,
    notifications_registered: Option<bool>,
}

/// Settings > Health and the agents' `get_health`: computed from state the app already has.
pub(crate) async fn health_report(app: &AppHandle) -> health::Report {
    let state = app.state::<AppState>();
    let snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    let seen = state.health.lock().unwrap().clone();
    let connected: Vec<_> = snapshot.accounts.iter().filter(|a| a.monitor_connected).map(|a| (a.provider, a.id.clone())).collect();
    let monitoring = state.monitor.health(&connected).await;
    let rclone_installed = settings.presence_remote.is_some()
        && tauri::async_runtime::spawn_blocking(crate::rclone::find).await.ok().flatten().is_some();
    #[cfg(windows)]
    let (app_enabled, system_enabled) = notify::toast_switches(&app.config().identifier);
    #[cfg(not(windows))]
    let (app_enabled, system_enabled) = (None, None);
    let n = &settings.notifications;
    let sharing = health::Sharing { folder: settings.presence_folder.clone(), remote: settings.presence_remote.clone(), rclone_installed, last: seen.sharing };
    health::checks(&health::Inputs {
        accounts: &snapshot.accounts,
        monitoring: &monitoring,
        refresh_minutes: settings.refresh_minutes,
        last_success: seen.last_success,
        door: door::health(),
        sharing: &sharing,
        roster: {
            let seen = state.roster_seen.lock().unwrap().clone();
            health::Roster { enabled: settings.account_roster, passphrase_saved: roster_passphrase_saved(), wrote: seen.wrote, read: seen.read,
                unreadable: seen.unreadable, waiting: roster_waiting(&state, &snapshot).len() }
        },
        journal: state.diagnostics.health(),
        notifications: health::Notifications { registered: seen.notifications_registered, app_enabled, system_enabled,
            any_wanted: n.usage || n.resets || n.switches || n.renewals || n.unexpected_use },
        auto_connect: settings.auto_connect_monitoring,
        vault_plaintext: accounts::plaintext_slots(),
        processes: tauri::async_runtime::spawn_blocking(crate::instance::processes).await.ok().flatten(),
        disk: tauri::async_runtime::spawn_blocking(disk_use).await.ok().flatten(),
        wsl: sources::wsl_status(),
    }, chrono::Utc::now())
}

/// Space on disk for Health, measured at most every few minutes: the flyout asks after every
/// refresh, and walking the folders each time would be wasted work.
fn disk_use() -> Option<health::Disk> {
    static LAST: Mutex<Option<(Instant, health::Disk)>> = Mutex::new(None);
    const FRESH: Duration = Duration::from_secs(300);
    if let Some((at, disk)) = *LAST.lock().unwrap() {
        if at.elapsed() < FRESH {
            return Some(disk);
        }
    }
    let data = settings::app_dir()?;
    let rollback = dirs::data_local_dir()?.join("SlopBar-Rollback");
    let disk = health::Disk::measure(&data, &rollback);
    *LAST.lock().unwrap() = Some((Instant::now(), disk));
    Some(disk)
}

#[tauri::command]
async fn get_health(app: AppHandle) -> health::Report {
    health_report(&app).await
}

/// Quits, then starts the installed executable again (the door's `restart_app`, Health's fix).
fn restart(app: &AppHandle) -> Result<(), String> {
    // One successor however many callers ask at once (parallel installs each ask).
    static RESTARTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RESTARTING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Ok(());
    }
    // The installed executable, even if this one was renamed aside for an update.
    let exe = std::env::current_exe().map_err(|e| e.to_string())?
        .with_file_name(crate::instance::APP_EXE);
    // A duplicate of an older build ignores the instance lock and would take the door over once
    // this one exits; it renews the same monitoring sessions, so it is ended rather than kept.
    crate::instance::end_duplicates();
    if let Err(e) = std::process::Command::new(&exe).arg("--after").arg(std::process::id().to_string()).spawn() {
        RESTARTING.store(false, std::sync::atomic::Ordering::Release);
        return Err(format!("restart failed: {e}"));
    }
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        h.exit(0);
    });
    Ok(())
}

#[tauri::command]
fn restart_app(app: AppHandle) -> Result<(), String> {
    restart(&app)
}

#[tauri::command]
fn get_usage(state: tauri::State<AppState>) -> Snapshot {
    state.snapshot.lock().unwrap().clone()
}

fn subscription_report(state: &AppState) -> subscriptions::Report {
    let snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    let local = state.codex_local.lock().unwrap().clone();
    let now = chrono::Utc::now();
    let mut report = subscriptions::report(&snapshot, &state.usage_history.lock().unwrap(), &settings, local.as_deref(), now);
    report.spend = Some(ledger::summary(&state.ledger.lock().unwrap(), &settings.spend, now));
    report
}

/// Charges renewals that have passed and this month's extra usage to the spend ledger.
fn record_spend(state: &AppState) {
    let report = subscription_report(state);
    let ledger = {
        let mut ledger = state.ledger.lock().unwrap();
        ledger::record(&mut ledger, &report, chrono::Utc::now()).then(|| ledger.clone())
    };
    if let Some(ledger) = ledger { let _ = ledger::save(&ledger); }
}

#[tauri::command]
fn get_subscriptions(state: tauri::State<AppState>) -> subscriptions::Report {
    subscription_report(&state)
}

/// Each account's weekly usage over the last 7 days, for the sparklines (see
/// `subscriptions::sparklines`), keyed `provider:id`.
#[tauri::command]
fn get_sparklines(state: tauri::State<AppState>) -> HashMap<String, Vec<(i64, f64)>> {
    subscriptions::sparklines(&state.usage_history.lock().unwrap(), chrono::Utc::now())
}

/// Esc in the flyout closes it.
#[tauri::command]
fn hide_panel(app: AppHandle) {
    panel::close(&app);
}

/// The plan notifications due now (see `plan_alerts`), each returned once. The app asks after
/// every refresh and words them (see `alerts`).
#[tauri::command]
fn take_plan_alerts(state: tauri::State<AppState>) -> Vec<plan_alerts::Alert> {
    plan_alerts_due(&state)
}

pub(crate) fn plan_alerts_due(state: &AppState) -> Vec<plan_alerts::Alert> {
    let snapshot = state.snapshot.lock().unwrap().clone();
    if snapshot.updated_at.is_none() { return vec![]; }
    let report = subscription_report(state);
    let prefs = state.settings.lock().unwrap().notifications.clone();
    let mut sent = state.plan_alerts.lock().unwrap();
    let due = plan_alerts::due(&report, &snapshot, &prefs, &mut sent, chrono::Utc::now());
    // Unsaved, they are still remembered until the app restarts.
    if !due.is_empty() { let _ = plan_alerts::save(&sent); }
    due
}

/// Runaway-agent (see `burn`) and reset-credit (see `reset_credits`) notifications due now, each
/// returned once. The app asks after every refresh and words them (see `alerts`).
#[tauri::command]
fn take_capacity_alerts(state: tauri::State<AppState>) -> serde_json::Value {
    capacity_alerts_due(&state)
}

pub(crate) fn capacity_alerts_due(state: &AppState) -> serde_json::Value {
    let burn = std::mem::take(&mut *state.burn_due.lock().unwrap());
    let snapshot = state.snapshot.lock().unwrap().clone();
    let enabled = state.settings.lock().unwrap().notifications.reset_credits;
    let credits = if snapshot.updated_at.is_some() {
        let mut sent = state.plan_alerts.lock().unwrap();
        let due = reset_credits::due(&snapshot.accounts, enabled, &mut sent, chrono::Utc::now());
        if !due.is_empty() { let _ = plan_alerts::save(&sent); }
        due
    } else { vec![] };
    serde_json::json!({ "burn": burn, "credits": credits })
}

/// Spend or switch, per Codex account at its limit with a reset credit, for the flyout.
#[tauri::command]
fn get_credit_advice(state: tauri::State<AppState>) -> Vec<reset_credits::CreditAdvice> {
    reset_credits::advise(&state.snapshot.lock().unwrap().accounts, chrono::Utc::now())
}

/// The Monday digest (see `digest`), once per week. The app asks after every refresh and words
/// it (see `alerts`).
#[tauri::command]
fn take_digest(state: tauri::State<AppState>) -> Option<digest::Digest> {
    digest_due(&state)
}

pub(crate) fn digest_due(state: &AppState) -> Option<digest::Digest> {
    if state.snapshot.lock().unwrap().updated_at.is_none() { return None; }
    let enabled = state.settings.lock().unwrap().notifications.digest;
    let local = chrono::Local::now().naive_local();
    digest::week_due(local)?;
    let report = subscription_report(state);
    let history = state.usage_history.lock().unwrap().clone();
    let mut sent = state.plan_alerts.lock().unwrap();
    let digest = digest::take(&report, &history, enabled, &mut sent, local, chrono::Utc::now());
    if digest.is_some() { let _ = plan_alerts::save(&sent); }
    digest
}

/// Writes the plan-renewal calendar now (see `calendar`); shared by Settings, the agents' door and
/// each refresh while it is turned on.
async fn write_calendar(app: &AppHandle) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    let report = subscription_report(&state);
    let settings = state.settings.lock().unwrap().clone();
    let (path, events) = tauri::async_runtime::spawn_blocking(move || calendar::export(&report, &settings, chrono::Utc::now()))
        .await.map_err(|e| e.to_string())??;
    Ok(serde_json::json!({ "path": path, "events": events }))
}

#[tauri::command]
async fn export_calendar(app: AppHandle) -> Result<serde_json::Value, String> {
    write_calendar(&app).await
}

/// Whether Settings may store `path` as the calendar's path.
#[tauri::command]
fn check_calendar_path(path: String) -> bool {
    calendar::valid_path(path.trim())
}

/// Where the calendar goes when no path is set.
#[tauri::command]
fn default_calendar_path() -> Option<String> {
    calendar::default_path().map(|p| p.to_string_lossy().into_owned())
}

/// Opens the provider's billing page in the browser. The address is fixed per provider, so the
/// window can only choose which of the two opens.
#[tauri::command]
async fn open_billing_page(provider: Provider) -> Result<(), String> {
    let url = subscriptions::billing_page(provider);
    tauri::async_runtime::spawn_blocking(move || std::process::Command::new("explorer.exe").arg(url).spawn().map(|_| ()))
        .await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// Writes a diagnostics export; shared by Settings and the agents' door.
async fn write_export(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let state = app.state::<AppState>();
    let recorder = state.diagnostics.clone();
    let snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || recorder.export(&snapshot, &settings))
        .await.map_err(|_| "diagnostic_storage")?.map_err(String::from)
}

#[tauri::command]
async fn export_diagnostics(app: AppHandle) -> Result<String, String> {
    let path = write_export(&app).await?;
    #[cfg(windows)]
    if let Some(parent) = path.parent() {
        let folder = parent.to_owned();
        // No shell or user-supplied command. Open the dedicated export folder.
        let _ = tauri::async_runtime::spawn_blocking(move || std::process::Command::new("explorer.exe").arg(folder).spawn()).await;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// An agent's switch: the same as Use in Settings, then the balancer's cooldown so the next poll
/// does not switch straight back. Returns the new snapshot and whether the account held.
async fn agent_switch(app: &AppHandle, provider: Provider, id: String) -> Result<serde_json::Value, String> {
    switch_to(app, provider, id.clone(), false).await?;
    app.state::<AppState>().last_switch.lock().unwrap().insert(provider, Instant::now());
    let snapshot = do_refresh(app).await;
    let active = snapshot.accounts.iter().find(|a| a.provider == provider && a.active).map(|a| a.id.clone());
    let mut result = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    result["switched_to_requested"] = (active.as_deref() == Some(id.as_str())).into();
    Ok(result)
}

/// One operation for an agent, through the door (see `door` and `mcp`). Each runs the same code as
/// the matching Settings action, so the windows and journal see it the same way.
async fn door_call(app: AppHandle, request: door::Request) -> Result<serde_json::Value, String> {
    use door::Request as R;
    fn value(x: impl Serialize) -> Result<serde_json::Value, String> {
        serde_json::to_value(x).map_err(|e| e.to_string())
    }
    match request.clone() {
        R::ExportDiagnostics => Ok(serde_json::json!({ "path": write_export(&app).await? })),
        R::GetUsage => value(app.state::<AppState>().snapshot.lock().unwrap().clone()),
        R::Refresh => value(do_refresh(&app).await),
        R::GetSettings => value(app.state::<AppState>().settings.lock().unwrap().clone()),
        R::GetSubscriptions => value(subscription_report(&app.state::<AppState>())),
        R::GetPlanAdvice => {
            let report = subscription_report(&app.state::<AppState>());
            let credits = reset_credits::advise(&app.state::<AppState>().snapshot.lock().unwrap().accounts, chrono::Utc::now());
            Ok(serde_json::json!({ "providers": report.advice, "reset_credits": credits, "generated_at": report.generated_at }))
        }
        R::GetBurnAlerts => {
            let state = app.state::<AppState>();
            let settings = state.settings.lock().unwrap().clone();
            let alerts = burn::seen(&state.burn_log.lock().unwrap(), chrono::Utc::now());
            Ok(serde_json::json!({ "alerts": value(alerts)?, "thresholds": settings.burn, "notify": settings.notifications.burn, "notify_every_hours": burn::NOTIFY_EVERY_H }))
        }
        R::GetUsageBreakdown { provider, account, days, group_by } => {
            if days.is_some_and(|d| !(1..=usage_index::LOOKBACK_DAYS as u32).contains(&d)) {
                return Err(format!("days must be 1-{}", usage_index::LOOKBACK_DAYS));
            }
            value(usage_breakdown(&app, usage_index::Filter { provider, account, days: days.unwrap_or(7), group_by: group_by.unwrap_or_default() }).await)
        }
        R::ExportCsv { folder } => value(write_csv(&app, folder).await?),
        R::ExportBackup { path, passphrase } => write_backup(&app, path, passphrase).await,
        R::RestoreBackup { path, passphrase, dry_run } => {
            let bytes = tauri::async_runtime::spawn_blocking(move || backup::read(&path)).await.map_err(|_| "backup_unreadable".to_string())??;
            value(apply_backup(&app, bytes, passphrase, dry_run).await?)
        }
        R::UpdateSettings { patch } => {
            let state = app.state::<AppState>();
            let _held = state.settings_lock.lock().await;
            let current = app.state::<AppState>().settings.lock().unwrap().clone();
            let next = settings::patched(&current, &patch, updates_enabled())?;
            let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
            for key in next.aliases.keys().filter(|k| !current.aliases.contains_key(*k)) {
                if !snapshot.accounts.iter().any(|a| format!("{}:{}", a.provider.name(), a.id) == *key) {
                    return Err(format!("no account {key:?}; alias keys are \"provider:id\" from get_usage"));
                }
            }
            store_settings(&app, next.clone()).await?;
            value(next)
        }
        R::SwitchAccount { provider, id } => agent_switch(&app, provider, id).await,
        R::GetHeadroom { provider, id } => {
            let state = app.state::<AppState>();
            let snapshot = state.snapshot.lock().unwrap().clone();
            let settings = state.settings.lock().unwrap().clone();
            let now = chrono::Utc::now();
            let picked: Vec<_> = snapshot.accounts.iter().filter(|a| match (&provider, &id) {
                (_, Some(id)) => a.id == *id && provider.map_or(true, |p| a.provider == p),
                (Some(p), None) => a.provider == *p && a.active,
                (None, None) => a.active,
            }).collect();
            if picked.is_empty() {
                return Err(if id.is_some() { "monitor_wrong_account".into() } else { "no_active_account".into() });
            }
            let accounts: Vec<_> = {
                let recent = state.recent.lock().unwrap();
                picked.into_iter().map(|a| headroom::headroom(a, &settings, &recent, now)).collect()
            };
            Ok(serde_json::json!({ "accounts": value(accounts)?, "updated_at": snapshot.updated_at }))
        }
        R::RecommendAccount { provider, expected_percent, hours, switch, lease_id } => {
            if expected_percent.is_some_and(|p| !(0.0..=100.0).contains(&p)) || hours.is_some_and(|h| !(0.0..=168.0).contains(&h)) {
                return Err("expected_percent must be 0-100 and hours 0-168".into());
            }
            let rec = recommend_unleased(&app, provider, expected_percent, hours, lease_id.as_deref());
            let mut result = value(&rec)?;
            result["switched"] = false.into();
            if let (true, headroom::Action::Switch, Some(id)) = (switch, rec.action, rec.recommended.clone()) {
                let done = agent_switch(&app, provider, id).await?;
                result["switched"] = true.into();
                result["switched_to_requested"] = done["switched_to_requested"].clone();
            }
            Ok(result)
        }
        R::LeaseAccount { provider, purpose, minutes, expected_percent, id } => {
            if expected_percent.is_some_and(|p| !(0.0..=100.0).contains(&p)) {
                return Err("expected_percent must be 0-100".into());
            }
            lease_account(&app, provider, &purpose, minutes, expected_percent, id, None).await
        }
        R::ReleaseLease { lease_id } => {
            let released = app.state::<AppState>().leases.lock().unwrap().release(&lease_id).ok_or("no_such_lease")?;
            leases_changed(&app).await;
            Ok(serde_json::json!({ "released": true, "lease": value(released)? }))
        }
        R::ListLeases => {
            prune_leases(&app).await;
            Ok(serde_json::json!({ "leases": value(app.state::<AppState>().leases.lock().unwrap().list())? }))
        }
        R::PickAccount { provider, pid, purpose, expected_percent } => {
            if expected_percent.is_some_and(|p| !(0.0..=100.0).contains(&p)) {
                return Err("expected_percent must be 0-100".into());
            }
            if !leases::process_alive(pid) {
                return Err("pid_not_running".into());
            }
            prune_leases(&app).await;
            let rec = recommend_unleased(&app, provider, expected_percent, None, None);
            let Some(id) = rec.recommended.clone() else { return Err(format!("no_account_free: {}", rec.summary)) };
            let label = rec.recommended_label.clone();
            let at_default = rec.ranking.iter().any(|c| c.id == id && c.active);
            if at_default && purpose.is_none() {
                return Ok(serde_json::json!({ "id": id, "label": label, "dir": null, "env_var": pins::env_var(provider), "lease_id": null, "note": null }));
            }
            let mut leased = lease_account(&app, provider, purpose.as_deref().unwrap_or("slopbar-run"), None, expected_percent, Some(id), Some(pid)).await?;
            leased["dir"] = leased["config_dir"].clone();
            leased["lease_id"] = leased["lease"]["lease_id"].clone();
            leased["note"] = if !at_default && leased["dir"].is_null() {
                "the recommended account's profile is not signed in yet, so this runs on the CLI's current sign-in".into()
            } else {
                serde_json::Value::Null
            };
            Ok(leased)
        }
        R::RecordSession { session_id, cwd, config_dir } => record_session(&app, session_id, cwd, config_dir).await,
        R::Turn { session_id, event, config_dir } => record_turn(&app, session_id, event, config_dir).await,
        R::AddAccount { provider } => {
            desktop_shown()?;
            // One pending sign-in per provider, so a retrying agent cannot flood the desktop.
            let pending = app.state::<AppState>().account_operations.begin(provider, "add-account")?;
            start_add_account(app.clone(), provider, Some(pending)).await?;
            Ok(serde_json::json!({ "started": true, "next": "The user completes the sign-in in the terminal that opened; the account then appears in get_usage." }))
        }
        R::ConnectMonitoring { provider, id } => {
            desktop_shown()?;
            connect_monitoring(app.clone(), provider, id).await?;
            Ok(serde_json::json!({ "started": true, "next": "The user completes the monitoring sign-in that opened; get_usage then shows monitor_connected." }))
        }
        R::DisconnectMonitoring { provider, id } => value(disconnect_monitoring(app.clone(), provider, id).await?),
        R::RemoveAccount { provider, id } => value(remove_account(app.clone(), provider, id).await?),
        R::RevokeAccountSessions { provider, id, confirm, remove_slot } => {
            if !confirm {
                let (connected, active) = session_facts(&app, provider, &id).ok_or("monitor_wrong_account")?;
                return Ok(serde_json::json!({ "revoked": false, "monitoring_connected": connected, "in_use": active,
                    "would_revoke_monitoring": connected, "would_remove_slot": remove_slot && !active, "cli_login": "managed_by_cli",
                    "next": "Nothing was revoked. confirm: true revokes SlopBar's monitoring sign-in for this account at the provider and forgets it here; remove_slot: true also removes the saved CLI login from SlopBar (not while in use). The CLI's own sign-in is managed by the CLI: sign it out there (claude /logout, codex logout)." }));
            }
            value(revoke_sessions(&app, provider, &id, remove_slot).await?)
        }
        R::ConnectMonitoringAll { confirm } => {
            if !confirm {
                let to_connect = bulk::unmonitored(&app.state::<AppState>().snapshot.lock().unwrap());
                return Ok(serde_json::json!({ "started": false, "to_connect": to_connect,
                    "next": "Nothing was started. With confirm: true the sign-ins open one at a time; the user signs in as each account named here." }));
            }
            desktop_shown()?;
            let queued = connect_all(&app).await?;
            Ok(serde_json::json!({ "started": !queued.is_empty(), "queued": queued,
                "next": "Each sign-in opens after the previous one finishes; the user signs in as the account named in Settings. get_usage shows monitor_connected as they complete." }))
        }
        R::SignInAll { confirm, monitoring, retry } => {
            if !confirm {
                return Ok(serde_json::json!({ "started": false, "plan": sign_in_plan(&app, monitoring),
                    "next": "Nothing was started. With confirm: true the sign-ins open one at a time, an email's back to back in one fresh browser profile that is closed and deleted when the email is done; the user signs in as each account named here and completes every page." }));
            }
            desktop_shown()?;
            // Retry from another computer: a run still going (one left waiting) gives way first.
            if retry {
                stop_and_wait(&app).await?;
            }
            let plan = sign_in_all(&app, monitoring).await?;
            Ok(serde_json::json!({ "started": !plan.is_empty(), "plan": plan,
                "next": "Each sign-in opens after the previous one finishes. get_usage shows the accounts and monitor_connected as they complete." }))
        }
        R::RemoveLapsedAccounts { confirm } => {
            if !confirm {
                let state = app.state::<AppState>();
                let snapshot = state.snapshot.lock().unwrap().clone();
                let settings = state.settings.lock().unwrap().clone();
                let plan = bulk::lapsed(&snapshot, &settings, chrono::Utc::now());
                return Ok(serde_json::json!({ "removed": false, "to_remove": plan.remove, "in_use": plan.in_use,
                    "next": "Nothing was removed. confirm: true removes to_remove and revokes their monitoring sign-ins; accounts in use are never removed." }));
            }
            Ok(remove_lapsed(&app).await)
        }
        R::ListProjectPins => value(pin_listing(&app)),
        R::SetProjectPin { folder, provider, id } => value(set_project_pin(app.clone(), folder, provider, id).await?),
        R::CheckForUpdates => value(check_for_updates(app.clone()).await?),
        R::InstallUpdate => {
            let latest = check_for_updates(app.clone()).await?.latest;
            if latest.is_none() {
                return Ok(serde_json::json!({ "installing": false, "reason": "SlopBar is up to date" }));
            }
            // Installing restarts SlopBar: reply first, then install.
            let h = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = updates::install(&h).await;
            });
            Ok(serde_json::json!({ "installing": true, "version": latest, "next": "SlopBar restarts into the new version shortly." }))
        }
        R::GetAppInfo => Ok(serde_json::json!({
            "version": app_version(app.clone()),
            "updates_enabled": updates_enabled(),
            "build": diagnostics::build_info(),
            // Which process answers the door (the installer quits exactly this one).
            "pid": std::process::id(),
        })),
        R::InspectWindow { window, page, screenshot } => {
            let target = match window {
                door::InspectTarget::Settings => inspect::Target::Settings,
                door::InspectTarget::Flyout => inspect::Target::Flyout,
                door::InspectTarget::Widget => inspect::Target::Widget,
                // Created for the look if it is closed (see inspect::inspect).
                door::InspectTarget::Switcher => inspect::Target::Switcher,
            };
            value(inspect::inspect(&app, target, page, screenshot).await?)
        }
        R::GetHealth => value(health_report(&app).await),
        R::GetRoster { sync } => Ok(roster_status(&app, sync).await),
        R::SyncRoster => Ok(roster_status(&app, true).await),
        R::StopSignIns => {
            let state = app.state::<AppState>();
            let running = state.bulk_connect.lock().unwrap().as_ref().is_some_and(|b| !b.finished);
            stop_running_sign_ins(&state);
            Ok(serde_json::json!({ "stopped": running }))
        }
        R::GetReadiness { wait_seconds, since } => {
            if let Some(wait) = wait_seconds {
                let since = since.unwrap_or_else(|| app.state::<AppState>().bulk_revision.load(std::sync::atomic::Ordering::SeqCst));
                wait_for_sign_in_change(&app, since, Duration::from_secs(u64::from(wait.min(45)))).await;
            }
            Ok(readiness(&app).await)
        }
        R::SetRosterPassphrase { passphrase } => set_roster_passphrase(&app, passphrase).await,
        R::ListSharingLocations => {
            let (folder, remote) = {
                let s = app.state::<AppState>().settings.lock().unwrap().clone();
                (s.presence_folder, s.presence_remote)
            };
            Ok(serde_json::json!({
                "folders": sync_folders().await,
                "rclone": rclone_remotes().await,
                "current": { "presence_folder": folder, "presence_remote": remote },
                "next": "Set presence_remote (an rclone remote name, no colon) or presence_folder (a folder path) with update_settings.",
            }))
        }
        R::OpenBillingPage { provider } => {
            open_billing_page(provider).await?;
            Ok(serde_json::json!({ "opened": subscriptions::billing_page(provider) }))
        }
        R::ExportCalendar => write_calendar(&app).await,
        R::OpenSettings => {
            show_settings(&app);
            Ok(serde_json::json!({ "opened": true }))
        }
        R::QuitApp | R::RestartApp => {
            let restart = matches!(request, R::RestartApp);
            // Reply first, then exit exactly as the tray's Quit does (journal flushed on Exit).
            if restart {
                self::restart(&app)?;
            } else {
                let h = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    h.exit(0);
                });
            }
            Ok(serde_json::json!({ "quitting": true, "restarting": restart }))
        }
    }
}

#[tauri::command]
fn record_frontend_error(state: tauri::State<AppState>, surface: diagnostics::Surface, kind: diagnostics::FrontendKind, line: Option<u32>, column: Option<u32>) {
    state.diagnostics.record(None, None, diagnostics::Event::Frontend { surface, kind, line, column });
}

#[tauri::command]
async fn refresh(app: AppHandle) -> Snapshot {
    do_refresh(&app).await
}

/// Settings > Data, "Rescan WSL": reads every distro once, starting stopped ones, then refreshes.
#[tauri::command]
async fn rescan_wsl(app: AppHandle) -> Snapshot {
    sources::request_wsl_rescan();
    do_refresh(&app).await
}

#[derive(Clone, Serialize)]
struct Switched {
    provider: Provider,
    from: Option<String>,
    to: Option<String>,
    auto: bool,
}

/// Display name of an account: its alias, else its email.
fn account_label(app: &AppHandle, provider: Provider, id: &str) -> Option<String> {
    let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
    snapshot
        .accounts
        .iter()
        .find(|a| a.provider == provider && a.id == id)
        .and_then(|a| a.alias.clone().or(a.email.clone()))
}

async fn switch_to(app: &AppHandle, provider: Provider, id: String, auto: bool) -> Result<(), String> {
    let found = app.state::<AppState>().sources.lock().unwrap().clone();
    let from = {
        let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
        snapshot.accounts.iter().find(|a| a.provider == provider && a.active).map(|a| a.id.clone())
    };
    let target = id.clone();
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(move || accounts::switch(&found, provider, &target))
        .await
        .map_err(|e| e.to_string()).and_then(|r| r);
    app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action {
        action: if auto { diagnostics::Action::SwitchAuto } else { diagnostics::Action::SwitchManual },
        code: diagnostics::Code::from_error(result.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
    result?;
    // The account left behind gets its project profile back at once, not at the next refresh.
    sync_pins(app, None).await;
    let event = Switched {
        provider,
        from: from.and_then(|f| account_label(app, provider, &f)),
        to: account_label(app, provider, &id),
        auto,
    };
    crate::alerts::on_switch(app, event.provider, event.to.as_deref(), event.auto);
    let _ = app.emit("account-switched", event);
    Ok(())
}

#[tauri::command]
async fn switch_account(app: AppHandle, provider: Provider, id: String) -> Result<Snapshot, String> {
    switch_to(&app, provider, id, false).await?;
    Ok(do_refresh(&app).await)
}

/// Opens a terminal running the CLI's own sign-in; the account appears once it finishes.
#[tauri::command]
async fn add_account(app: AppHandle, provider: Provider) -> Result<(), String> {
    start_add_account(app, provider, None).await
}

/// Starts the CLI sign-in; `pending` (held until it finishes) keeps a second one from starting.
async fn start_add_account(app: AppHandle, provider: Provider, pending: Option<monitoring::Operation>) -> Result<(), String> {
    let found = {
        let known = app.state::<AppState>().sources.lock().unwrap().clone();
        if known.is_empty() {
            tauri::async_runtime::spawn_blocking(sources::discover).await.unwrap_or_default()
        } else {
            known
        }
    };
    // A fresh profile of its own (who signs in is not known yet), held until the sign-in ends.
    let session = signin_session(&app);
    let options = signin_options(&app, None, session.as_deref());
    // Probing for the CLI runs wsl.exe / where.exe; keep it off the async runtime.
    let job = tauri::async_runtime::spawn_blocking(move || accounts::start_login_observed(provider, &found, None, options))
        .await
        .map_err(|e| e.to_string())??;
    tauri::async_runtime::spawn(async move {
        let _pending = pending;
        let added = job.wait(&http_client()).await;
        drop(session);
        if let Some(slot) = added {
            forget_pending(slot.provider, &slot.id);
            let _ = app.emit("account-added", (slot.provider, slot.email));
            do_refresh(&app).await;
        }
    });
    Ok(())
}

/// A brand-new browser profile for a sign-in, or for an email's run of them, when the user chose
/// those (see `signin::Session`). Dropping the last handle closes its browser and deletes it.
fn signin_session(app: &AppHandle) -> Option<Arc<signin::Session>> {
    let window = app.state::<AppState>().settings.lock().unwrap().signin_window;
    (window == signin::Window::Profile).then(signin::Session::create).flatten().map(Arc::new)
}

/// Where a sign-in as `email` opens its page, from the user's choice (see `signin`): `session`'s
/// profile, a private window, or the CLI's default browser.
fn signin_options(app: &AppHandle, email: Option<&str>, session: Option<&signin::Session>) -> accounts::LoginOptions {
    let window = app.state::<AppState>().settings.lock().unwrap().signin_window;
    accounts::LoginOptions { email: email.map(str::to_owned), target: signin::Target::choose(window, session) }
}

/// A restored account that is signed in again is no longer pending.
fn forget_pending(provider: Provider, id: &str) {
    let mut list = bulk::load_pending();
    let before = list.len();
    list.retain(|p| !(p.provider == provider && p.id == id));
    if list.len() != before {
        let _ = bulk::save_pending(&list);
    }
}

/// Signs a restored account in again, as "Add account" does, but only as that account: a login
/// of any other account is not saved and is revoked, the way monitoring rejects a wrong account.
/// Returns once the user is done.
async fn add_expected(app: &AppHandle, expected: &bulk::Step, session: Option<&signin::Session>) -> Result<(), String> {
    let provider = expected.target.provider;
    let _operation = app.state::<AppState>().account_operations.begin(provider, "add-account")?;
    let _sign_in = app.state::<AppState>().account_operations.begin_key(monitoring::SIGN_IN)?;
    let found = app.state::<AppState>().sources.lock().unwrap().clone();
    let options = signin_options(app, expected.email.as_deref(), session);
    let job = tauri::async_runtime::spawn_blocking(move || accounts::start_login_observed(provider, &found, None, options))
        .await.map_err(|_| "monitor_login_failed".to_string())??;
    let client = http_client();
    let login = job.wait_isolated(&client).await.ok_or("monitor_login_failed")?;
    // The slot id names the account and, for Claude, its organization (server-verified above).
    if login.provider != provider || login.id != expected.target.id {
        // Never a token a stored login or monitor holds; this fresh login holds none of those.
        app.state::<AppState>().monitor.discard_unstored_login(&login, &accounts::load_slots(), &client).await;
        return Err(wrong_account(provider, expected.email.as_deref(), login.email.as_deref()));
    }
    accounts::store_login(&login).map_err(|_| "monitor_storage".to_string())?;
    forget_pending(provider, &login.id);
    let _ = app.emit("account-added", (login.provider, login.email.clone()));
    refresh_soon(app);
    Ok(())
}

/// A refresh after a sign-in, not waited for: one can take most of a minute (finding sign-ins,
/// sharing), and "Sign in all accounts" opens its next sign-in meanwhile. Asked for again while
/// one waits to start, it is that one.
fn refresh_soon(app: &AppHandle) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WAITING: AtomicBool = AtomicBool::new(false);
    if WAITING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // After any refresh running now, which may have read before the change.
        drop(app.state::<AppState>().refresh_lock.lock().await);
        WAITING.store(false, Ordering::SeqCst);
        do_refresh(&app).await;
    });
}

/// Why a sign-in was not the expected account. A Codex account is one ChatGPT workspace, so the
/// right email with another id means the person picked another workspace on the page.
fn wrong_account(provider: Provider, expected: Option<&str>, signed_in: Option<&str>) -> String {
    let same_email = matches!((expected, signed_in), (Some(a), Some(b)) if a.trim().eq_ignore_ascii_case(b.trim()));
    if provider == Provider::Codex && same_email { "monitor_wrong_workspace" } else { "monitor_wrong_account" }.into()
}

/// A one-time login owns its own refresh-token chain; it never changes the active CLI.
#[tauri::command]
async fn connect_monitoring(app: AppHandle, provider: Provider, id: String) -> Result<(), String> {
    let key = format!("{}:{id}", provider.name());
    auto_monitor_saved(&app, |t| t.manual(&key, true));
    start_monitoring_login(app, provider, id, false, None).await.map(|_| ())
}

/// Connect monitoring's sign-in, started by the user or on its own (`auto`, see `auto_monitor`).
/// One sign-in is open at a time across the app (`monitoring::SIGN_IN`); the returned task
/// ends with its outcome once the user is done.
async fn start_monitoring_login(app: AppHandle, provider: Provider, id: String, auto: bool, session: Option<Arc<signin::Session>>) -> Result<tauri::async_runtime::JoinHandle<Result<(), String>>, String> {
    let operation = app.state::<AppState>().account_operations.begin(provider, &id)?;
    let sign_in = app.state::<AppState>().account_operations.begin_key(monitoring::SIGN_IN)?;
    let expected = accounts::load_slots().into_iter().find(|s| s.provider == provider && s.id == id)
        .ok_or("monitor_wrong_account")?;
    let key = monitoring::key(&expected);
    let started = Instant::now();
    app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectStarted, code: diagnostics::Code::Ok, elapsed_ms: 0 });
    let found = app.state::<AppState>().sources.lock().unwrap().clone();
    let context = diagnostics::Context { recorder: app.state::<AppState>().diagnostics.clone(), provider, id: id.clone() };
    // The email group's profile in "Sign in all accounts", else a fresh one of its own.
    let session = session.or_else(|| signin_session(&app));
    let options = signin_options(&app, expected.email.as_deref(), session.as_deref());
    let job = tauri::async_runtime::spawn_blocking(move || accounts::start_login_observed(provider, &found, Some(context), options)).await;
    let job = match job {
        Ok(Ok(job)) => job,
        _ => {
            app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectFinished, code: diagnostics::Code::LoginFailed, elapsed_ms: diagnostics::millis(started) });
            return Err("monitor_login_failed".into());
        }
    };
    if auto {
        let name = app.state::<AppState>().snapshot.lock().unwrap().accounts.iter()
            .find(|a| a.provider == provider && a.id == id).and_then(|a| a.alias.clone().or_else(|| a.email.clone()));
        crate::alerts::on_monitor_auto_started(&app, name.as_deref());
        let _ = app.emit("monitoring-auto-started", serde_json::json!({ "provider": provider, "id": id, "name": name }));
    }
    Ok(tauri::async_runtime::spawn(async move {
        let operation = operation;
        let sign_in = sign_in;
        let outcome = match job.wait_isolated(&http_client()).await {
            Some(login) => {
                let slots = accounts::load_slots();
                if slots.iter().any(|s| s.provider == provider && s.id == id) {
                    app.state::<AppState>().monitor.connect_replacing(&expected, &login, &slots, &http_client()).await
                        .map_err(|e| if e == "monitor_wrong_account" { wrong_account(provider, expected.email.as_deref(), login.email.as_deref()) } else { e })
                } else {
                    let mut known = slots;
                    known.push(expected.clone());
                    app.state::<AppState>().monitor.discard_unstored_login(&login, &known, &http_client()).await;
                    Err("monitor_wrong_account".into())
                }
            }
            None => Err("monitor_login_failed".into()),
        };
        drop(session);
        drop(operation);
        drop(sign_in);
        if auto {
            let ok = outcome.is_ok();
            auto_monitor_saved(&app, |t| t.finished(&key, ok, chrono::Utc::now()));
        }
        app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectFinished,
            code: diagnostics::Code::from_error(outcome.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
        let _ = app.emit("monitoring-login", serde_json::json!({ "provider": provider, "id": id, "error": outcome.as_ref().err() }));
        // Preserve last-known data, but allow the new login to bypass an earlier error's backoff,
        // without holding up whatever waits for this sign-in (see `refresh_soon`).
        let h = app.clone();
        tauri::async_runtime::spawn(async move {
            {
                let state = h.state::<AppState>();
                let _refresh = state.refresh_lock.lock().await;
                providers::retry_now(&mut state.usage_cache.lock().unwrap(), &key);
            }
            refresh_soon(&h);
        });
        outcome
    }))
}

/// Updates "Connect monitoring for all" and tells Settings.
fn publish_bulk(app: &AppHandle, change: impl FnOnce(&mut BulkConnect)) {
    let state = app.state::<AppState>();
    let mut running = state.bulk_connect.lock().unwrap();
    let mut run = None;
    if let Some(bulk) = running.as_mut() {
        change(bulk);
        bulk.revision = state.bulk_revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let _ = app.emit("bulk-monitoring", bulk.clone());
        run = bulk.by_email.then(|| roster_run(bulk, chrono::Utc::now()));
    }
    drop(running);
    state.bulk_changed.notify_waiters();
    // The user's other computers follow "Sign in all accounts" through the roster.
    if let Some(run) = run {
        *state.roster_run.lock().unwrap() = Some(run);
        let settings = state.settings.lock().unwrap().clone();
        if settings.account_roster {
            let h = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = h.state::<AppState>();
                let accounts = state.snapshot.lock().unwrap().accounts.clone();
                sync_roster(&state, &settings, accounts, false, false).await;
            });
        }
    }
}

/// A run of "Sign in all accounts" as the roster carries it (see `roster::Run`).
fn roster_run(b: &BulkConnect, at: chrono::DateTime<chrono::Utc>) -> roster::Run {
    let word = |v: serde_json::Value| v.as_str().unwrap_or_default().to_string();
    roster::Run {
        revision: b.revision, at, total: b.total, done: b.done, emails: b.emails, email: b.email, finished: b.finished,
        still_waiting: b.still_waiting, monitoring: b.monitoring, error: b.error.clone(),
        steps: b.steps.iter().map(|s| roster::RunStep {
            provider: s.target.provider, account: s.target.account.clone(),
            kind: word(serde_json::json!(s.kind)), state: word(serde_json::json!(s.state)), error: s.error.clone(),
        }).collect(),
    }
}

/// Returns once the run has changed since `revision` (see `BulkConnect::revision`), or after
/// `wait`: an agent elsewhere follows a run without polling.
async fn wait_for_sign_in_change(app: &AppHandle, revision: u64, wait: Duration) {
    let state = app.state::<AppState>();
    let until = tokio::time::Instant::now() + wait;
    loop {
        let changed = state.bulk_changed.notified();
        tokio::pin!(changed);
        // Listening before looking, so a change between the two is never missed.
        changed.as_mut().enable();
        if state.bulk_revision.load(std::sync::atomic::Ordering::SeqCst) > revision {
            return;
        }
        if tokio::time::timeout_at(until, changed).await.is_err() {
            return;
        }
    }
}

/// Queues a monitoring sign-in for every account without one (see `run_sign_ins`). Returns the
/// queued accounts.
async fn connect_all(app: &AppHandle) -> Result<Vec<bulk::Target>, String> {
    let groups = bulk::sign_in_plan(&app.state::<AppState>().snapshot.lock().unwrap(), &[], true);
    let targets = groups.iter().flat_map(|g| g.steps.iter().map(|s| s.target.clone())).collect();
    run_sign_ins(app, groups, false, true)?;
    Ok(targets)
}

/// "Sign in all accounts": signs every pending account in again and, with `monitoring`, connects
/// monitoring for each account without it, one email at a time (see `bulk::sign_in_plan`).
/// Returns the plan.
async fn sign_in_all(app: &AppHandle, monitoring: bool) -> Result<Vec<bulk::Group>, String> {
    let groups = sign_in_plan(app, monitoring);
    run_sign_ins(app, groups.clone(), true, monitoring)?;
    Ok(groups)
}

/// Retry (the stop and "still waiting" notifications, Settings > Accounts): stops the run if one
/// is going, then starts the same kind again. Its plan is made afresh from what is still missing,
/// so what already worked is not asked for again.
async fn retry_run(app: &AppHandle) -> Result<(), String> {
    let last = app.state::<AppState>().bulk_connect.lock().unwrap().clone();
    stop_and_wait(app).await?;
    match last {
        Some(b) if !b.by_email => connect_all(app).await.map(|_| ()),
        Some(b) => sign_in_all(app, b.monitoring).await.map(|_| ()),
        None => sign_in_all(app, true).await.map(|_| ()),
    }
}

fn sign_in_plan(app: &AppHandle, monitoring: bool) -> Vec<bulk::Group> {
    let state = app.state::<AppState>();
    let snapshot = state.snapshot.lock().unwrap().clone();
    bulk::sign_in_plan(&snapshot, &pending_accounts(&state, &snapshot), monitoring)
}

/// Accounts to sign in here: those a restored backup named, and those other machines' rosters
/// name (see `roster::waiting`).
fn pending_accounts(state: &AppState, snapshot: &Snapshot) -> Vec<bulk::Pending> {
    let mut list = bulk::load_pending();
    bulk::add_pending(&mut list, roster_waiting(state, snapshot));
    list
}

/// The accounts other machines' rosters name that are not signed in here, while the roster is on.
fn roster_waiting(state: &AppState, snapshot: &Snapshot) -> Vec<bulk::Pending> {
    if !state.settings.lock().unwrap().account_roster {
        return vec![];
    }
    roster::waiting(&state.roster_seen.lock().unwrap(), &snapshot.accounts)
}

/// A task that is aborted when this is dropped, as a stopped "Sign in all accounts" drops its
/// open sign-in.
struct AbortOnDrop<T>(tauri::async_runtime::JoinHandle<T>);

impl<T> std::future::Future for AbortOnDrop<T> {
    type Output = tauri::Result<T>;
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Stops the run if one is going and waits for it to end.
async fn stop_and_wait(app: &AppHandle) -> Result<(), String> {
    let going = || app.state::<AppState>().bulk_connect.lock().unwrap().as_ref().is_some_and(|b| !b.finished);
    if going() {
        stop_running_sign_ins(&app.state::<AppState>());
        let until = Instant::now() + Duration::from_secs(20);
        while going() {
            if Instant::now() > until {
                return Err("monitor_busy".into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    Ok(())
}

/// Resolves once `session`'s browser, having opened for any of the email's sign-ins, is closed:
/// the user closed the window, so the sign-in can no longer finish. Never resolves without one.
async fn browser_closed(session: Option<&signin::Session>) {
    let Some(session) = session else { return std::future::pending().await };
    while !session.browser_closed() {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Resolves once a sign-in page opened in `session`'s profile at or after `since`; never without one.
async fn page_opened(session: Option<&signin::Session>, since: std::time::SystemTime) {
    let Some(session) = session else { return std::future::pending().await };
    while !session.page_opened_since(since) {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// What a sign-in is, as an email's page lists it: "Claude", "Codex monitoring".
fn step_name(lang: &str, step: &bulk::Step) -> String {
    let provider = match step.target.provider { Provider::Claude => "Claude", Provider::Codex => "Codex" };
    let key = if step.kind == bulk::Kind::Add { "signinPageStepAdd" } else { "signinPageStepConnect" };
    crate::i18n::t(lang, key, &[("provider", provider)])
}

/// How long a sign-in may take to open its page (Codex's app server can be a cold start).
const NO_PAGE_LIMIT: Duration = Duration::from_secs(40);
/// How long a sign-in may wait on the person before they are asked whether to retry it.
const STILL_WAITING: Duration = Duration::from_secs(5 * 60);

/// Watches the open sign-in: resolves with `signin_no_page` when its page never opened in the
/// email's profile (only a profile can tell), and, once it has waited long, asks the person at the
/// desktop whether to retry it (and never resolves after that).
async fn watch_step(app: &AppHandle, session: Option<&signin::Session>, since: std::time::SystemTime, account: &str) -> String {
    let started = tokio::time::Instant::now();
    if let Some(session) = session {
        tokio::time::sleep(NO_PAGE_LIMIT).await;
        if !session.page_opened_since(since) {
            return "signin_no_page".into();
        }
    }
    tokio::time::sleep_until(started + STILL_WAITING).await;
    publish_bulk(app, |b| b.still_waiting = true);
    sign_in_choice(app, "signInStillWaiting", &[("account", account)], true);
    std::future::pending().await
}

/// Runs the sign-ins one terminal at a time, each started once the previous one finished, an
/// email's sign-ins back to back in one fresh browser profile so Google is asked once for all of
/// them; the profile's browser is closed and the profile deleted when the email's turn ends.
/// Stops at the first that fails (the user closed it, closed the browser, signed in as another
/// account, its page never opened, or Stop was pressed). Every step's outcome stays in `steps`.
fn run_sign_ins(app: &AppHandle, groups: Vec<bulk::Group>, by_email: bool, monitoring: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let total = groups.iter().map(|g| g.steps.len()).sum();
    let stop = Arc::new(tokio::sync::Notify::new());
    {
        let mut running = state.bulk_connect.lock().unwrap();
        if running.as_ref().is_some_and(|b| !b.finished) {
            return Err("monitor_busy".into());
        }
        let steps = groups.iter().flat_map(|g| g.steps.iter())
            .map(|s| StepRecord { target: s.target.clone(), kind: s.kind, state: StepState::Pending, error: None, started_at: None, page_at: None, ended_at: None }).collect();
        *running = Some(BulkConnect { total, done: 0, current: None, kind: None, emails: if by_email { groups.len() } else { 0 }, email: 0,
            finished: total == 0, error: None, steps, by_email, monitoring, still_waiting: false, revision: 0 });
        *state.bulk_stop.lock().unwrap() = Some(stop.clone());
    }
    publish_bulk(app, |_| {});
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        let lang = crate::i18n::resolve(&h.state::<AppState>().settings.lock().unwrap().language);
        let mut error = None;
        let mut done = 0;
        let mut position = 0;
        let mark = |at: usize, state: StepState, error: Option<String>| publish_bulk(&h, |b| {
            if let Some(step) = b.steps.get_mut(at) {
                let now = Some(chrono::Utc::now());
                match state {
                    StepState::Running => step.started_at = now,
                    StepState::Pending => {}
                    _ => step.ended_at = now,
                }
                step.state = state;
                step.error = error;
            }
        });
        // Accounts this run signed in: one only just added has its monitoring sign-in next, though
        // the refresh that lists it may still be running.
        let mut added = std::collections::HashSet::new();
        'groups: for (index, group) in groups.iter().enumerate() {
            // Made at the email's first sign-in that still has to run, ended with its turn.
            let mut group_session: Option<Option<Arc<signin::Session>>> = None;
            // What the email's page shows of its turn (see `signin::Session::show`).
            let mut page = signin::Progress {
                steps: group.steps.iter().map(|s| (step_name(&lang, s), signin::PageStep::Pending)).collect(),
                at: 0,
                total,
            };
            for (in_group, step) in group.steps.iter().enumerate() {
                let target = &step.target;
                let at = position;
                position += 1;
                publish_bulk(&h, |b| {
                    b.done = done;
                    b.current = Some(target.clone());
                    b.kind = by_email.then_some(step.kind);
                    b.email = if by_email { index + 1 } else { 0 };
                    b.still_waiting = false;
                });
                done += 1;
                // Done (or removed) since the queue started: nothing to do.
                let still = {
                    let state = h.state::<AppState>();
                    let snapshot = state.snapshot.lock().unwrap();
                    let account = snapshot.accounts.iter().find(|a| a.provider == target.provider && a.id == target.id);
                    match step.kind {
                        bulk::Kind::Add => account.is_none(),
                        bulk::Kind::Connect => account.map_or(added.contains(&(target.provider, &target.id)), |a| !a.monitor_connected),
                    }
                };
                if !still {
                    mark(at, StepState::Skipped, None);
                    page.steps[in_group].1 = signin::PageStep::Skipped;
                    continue;
                }
                // Locked or left since the queue started: stop rather than open a page nobody sees.
                if let Err(problem) = desktop_shown() {
                    done -= 1;
                    mark(at, StepState::Failed, Some(problem.clone()));
                    error = Some(problem);
                    break 'groups;
                }
                mark(at, StepState::Running, None);
                // Another sign-in is open (one the user or automatic monitoring started): wait for it.
                let free = async {
                    while h.state::<AppState>().account_operations.busy(monitoring::SIGN_IN) {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                };
                let stopped = tokio::select! {
                    _ = free => false,
                    _ = stop.notified() => true,
                };
                let session = group_session.get_or_insert_with(|| {
                    let session = signin_session(&h);
                    if let (Some(s), Some(email)) = (session.as_deref(), group.email.as_deref()) {
                        s.label(email, &lang);
                    }
                    session
                }).clone();
                page.at = done;
                page.steps[in_group].1 = signin::PageStep::Opening;
                let show = |session: Option<&signin::Session>, page: &signin::Progress| if let Some(s) = session { s.show(page) };
                show(session.as_deref(), &page);
                let since = std::time::SystemTime::now();
                let run = async {
                    match step.kind {
                        bulk::Kind::Add => add_expected(&h, step, session.as_deref()).await,
                        bulk::Kind::Connect => {
                            // The user asked for these, so they count as manual starts, never automatic ones.
                            let key = format!("{}:{}", target.provider.name(), target.id);
                            auto_monitor_saved(&h, |t| t.manual(&key, true));
                            match start_monitoring_login(h.clone(), target.provider, target.id.clone(), false, session.clone()).await {
                                // Stopping drops this future, and with it the sign-in.
                                Ok(task) => AbortOnDrop(task).await.unwrap_or_else(|_| Err("monitor_login_failed".into())),
                                Err(e) => Err(e),
                            }
                        }
                    }
                };
                // Closing the browser after the email's previous sign-in finished stops nothing.
                if let Some(s) = session.as_deref() {
                    s.rearm();
                }
                // Whoever sits at this desktop (often brought here by an agent on another
                // computer) is told which account and which page comes next.
                let account = target.account.clone().unwrap_or_default();
                if by_email && !stopped {
                    let (g, e, i, n) = ((index + 1).to_string(), groups.len().to_string(), done.to_string(), total.to_string());
                    let key = if step.kind == bulk::Kind::Add { "signInAllAdding" } else { "signInAllConnecting" };
                    sign_in_notice(&h, key, &[("g", &g), ("e", &e), ("i", &i), ("n", &n), ("account", &account)]);
                }
                let mut run = Box::pin(run);
                let outcome = if stopped {
                    Err("signin_stopped".into())
                } else {
                    let closed = browser_closed(session.as_deref());
                    let watch = watch_step(&h, session.as_deref(), since, &account);
                    let opened = page_opened(session.as_deref(), since);
                    tokio::pin!(closed, watch, opened);
                    let mut opening = true;
                    loop {
                        tokio::select! {
                            outcome = &mut run => break outcome,
                            _ = stop.notified() => break Err("signin_stopped".into()),
                            // Closing the window once the provider's page says "done" is fine: the
                            // sign-in may still be saving (Codex's CLI can take a while), so it gets up to
                            // two minutes to finish before the queue stops.
                            _ = &mut closed => break tokio::time::timeout(SIGN_IN_GRACE, &mut run).await
                                .unwrap_or_else(|_| Err("signin_window_closed".into())),
                            e = &mut watch => break Err(e),
                            _ = &mut opened, if opening => {
                                opening = false;
                                publish_bulk(&h, |b| if let Some(s) = b.steps.get_mut(at) { s.page_at = Some(chrono::Utc::now()) });
                                page.steps[in_group].1 = signin::PageStep::Open;
                                show(session.as_deref(), &page);
                            }
                        }
                    }
                };
                match outcome {
                    Ok(()) => {
                        mark(at, StepState::Done, None);
                        if step.kind == bulk::Kind::Add {
                            added.insert((target.provider, &target.id));
                        }
                        page.steps[in_group].1 = signin::PageStep::Done;
                        show(session.as_deref(), &page);
                    }
                    Err(e) => {
                        done -= 1;
                        mark(at, StepState::Failed, Some(e.clone()));
                        error = Some(e);
                        drop(run);
                        drop(group_session);
                        drop(session);
                        break 'groups;
                    }
                }
            }
            // The last handle: closes the browser and deletes the profile (see `signin::Session`).
            drop(group_session);
        }
        if by_email && total > 0 {
            match &error {
                Some(e) => {
                    let known = e.starts_with("signin_") || e.starts_with("monitor_") || e.starts_with("desktop_");
                    let reason = if known { crate::i18n::t(&lang, e, &[]) } else { e.clone() };
                    sign_in_choice(&h, "bulkConnectStopped", &[("done", &done.to_string()), ("n", &total.to_string()), ("error", &reason)], false);
                }
                None => sign_in_notice(&h, "signInAllDone", &[("n", &total.to_string())]),
            }
        }
        publish_bulk(&h, |b| {
            b.done = done;
            b.current = None;
            b.kind = None;
            b.finished = true;
            b.error = error;
            b.still_waiting = false;
        });
    });
    Ok(())
}

/// How long a sign-in may go on finishing after its browser window was closed.
const SIGN_IN_GRACE: Duration = Duration::from_secs(120);

/// The notification text for the person at this desktop during "Sign in all accounts", in the
/// words Settings > Accounts uses: (title, body, language).
fn sign_in_text(app: &AppHandle, key: &str, vars: &[(&str, &str)]) -> (String, String, String) {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let lang = crate::i18n::resolve(&settings.language);
    let place = if settings.signin_window == signin::Window::System { "signInWhereTerminal" } else { "signInWhereBrowser" };
    let place = crate::i18n::t(&lang, place, &[]);
    let mut vars = vars.to_vec();
    vars.push(("where", &place));
    (crate::i18n::t(&lang, "signInAll", &[]), crate::i18n::t(&lang, key, &vars), lang)
}

/// A notification for the person at this desktop; clicking it opens Settings > Accounts.
fn sign_in_notice(app: &AppHandle, key: &str, vars: &[(&str, &str)]) {
    let (title, body, _) = sign_in_text(app, key, vars);
    let _ = notify::notify(app.clone(), title, body, Some("accounts".into()), None);
}

/// The same with a Retry button (see `retry_run`) and, while the run goes on, a Stop one.
fn sign_in_choice(app: &AppHandle, key: &str, vars: &[(&str, &str)], running: bool) {
    let (title, body, lang) = sign_in_text(app, key, vars);
    let mut buttons = vec![(crate::i18n::t(&lang, "signInRetry", &[]), "retry")];
    if running {
        buttons.push((crate::i18n::t(&lang, "signInAllStop", &[]), "stop"));
    }
    let h = app.clone();
    let _ = notify::notify_choice(app.clone(), title, body, buttons, move |action| match action {
        Some("retry") => {
            let h = h.clone();
            tauri::async_runtime::spawn(async move {
                // A failure shows in Settings and readiness like any run that did not start.
                let _ = retry_run(&h).await;
            });
        }
        Some("stop") => stop_running_sign_ins(&h.state::<AppState>()),
        _ => show_settings_page(&h, Some("accounts".into())),
    });
}

#[tauri::command]
async fn connect_monitoring_all(app: AppHandle) -> Result<Vec<bulk::Target>, String> {
    connect_all(&app).await
}

/// What the bulk actions would act on now, and how a running "Connect for all" is doing.
#[tauri::command]
fn bulk_status(state: tauri::State<AppState>) -> serde_json::Value {
    let snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    serde_json::json!({
        "unmonitored": bulk::unmonitored(&snapshot),
        "lapsed": bulk::lapsed(&snapshot, &settings, chrono::Utc::now()),
        "connecting": state.bulk_connect.lock().unwrap().clone(),
        // "Sign in all accounts" with monitoring; without it, only the `add` steps run.
        "sign_in": bulk::sign_in_plan(&snapshot, &pending_accounts(&state, &snapshot), true),
        "roster_waiting": roster_waiting(&state, &snapshot).len(),
        "signin_profiles": signin::count(),
    })
}

#[tauri::command]
async fn sign_in_all_accounts(app: AppHandle, monitoring: bool) -> Result<Vec<bulk::Group>, String> {
    sign_in_all(&app, monitoring).await
}

/// "Clear old sign-in profiles": deletes the profiles earlier versions kept per email, and any a
/// stopped SlopBar left, with their cookies (see `signin::cleanup`).
#[tauri::command]
async fn forget_signin_profiles() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(signin::cleanup).await.map_err(|_| "signin_profiles_busy".to_string())?
}

/// Stops a running "Sign in all accounts" or "Connect monitoring for all": the open sign-in is
/// closed with its browser, and nothing after it starts.
#[tauri::command]
fn stop_sign_ins(state: tauri::State<AppState>) {
    stop_running_sign_ins(&state);
}

fn stop_running_sign_ins(state: &AppState) {
    if let Some(stop) = state.bulk_stop.lock().unwrap().as_ref() {
        stop.notify_one();
    }
}

/// Settings' Retry: see `retry_run`.
#[tauri::command]
async fn retry_sign_ins(app: AppHandle) -> Result<(), String> {
    retry_run(&app).await
}

/// Removes every account whose plan has ended, as each row's remove does; accounts in use stay.
async fn remove_lapsed(app: &AppHandle) -> serde_json::Value {
    let plan = {
        let state = app.state::<AppState>();
        let snapshot = state.snapshot.lock().unwrap().clone();
        let settings = state.settings.lock().unwrap().clone();
        bulk::lapsed(&snapshot, &settings, chrono::Utc::now())
    };
    let (mut removed, mut failed) = (Vec::new(), Vec::new());
    for target in &plan.remove {
        match remove_one(app, target.provider, &target.id).await {
            Ok(()) => removed.push(target.clone()),
            Err(error) => failed.push(serde_json::json!({ "account": target, "error": error })),
        }
    }
    let snapshot = do_refresh(app).await;
    serde_json::json!({ "removed": removed, "failed": failed, "in_use": plan.in_use, "snapshot": snapshot })
}

#[tauri::command]
async fn remove_lapsed_accounts(app: AppHandle) -> serde_json::Value {
    remove_lapsed(&app).await
}

#[tauri::command]
async fn remove_account(app: AppHandle, provider: Provider, id: String) -> Result<Snapshot, String> {
    remove_one(&app, provider, &id).await?;
    Ok(do_refresh(&app).await)
}

/// Removes a saved account that is not in use: its monitoring sign-in, its project pins and its
/// profile's login (the rest of the profile folder stays), then the saved login.
async fn remove_one(app: &AppHandle, provider: Provider, id: &str) -> Result<(), String> {
    let id = id.to_string();
    let operation = app.state::<AppState>().account_operations.begin(provider, &id)?;
    let active = {
        let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
        snapshot.accounts.iter().any(|a| a.provider == provider && a.id == id && a.active)
    };
    if active {
        // It would be captured again from the CLI on the next refresh.
        return Err("active".into());
    }
    if let Some(slot) = accounts::load_slots().into_iter().find(|s| s.provider == provider && s.id == id) {
        app.state::<AppState>().monitor.disconnect(&slot, &http_client()).await?;
        app.state::<AppState>().usage_cache.lock().unwrap().remove(&monitoring::key(&slot));
    }
    {
        let state = app.state::<AppState>();
        let _held = state.settings_lock.lock().await;
        let current = state.settings.lock().unwrap().clone();
        if current.project_pins.iter().any(|p| p.provider == provider && p.id == id) {
            let project_pins = current.project_pins.iter().filter(|p| !(p.provider == provider && p.id == id)).cloned().collect();
            store_settings(app, Settings { project_pins, ..current }).await?;
        }
    }
    if let Some(root) = accounts::profiles_root() {
        accounts::withdraw_profile(provider, &root.join(accounts::profile_name(provider, &id))).map_err(|e| e.to_string())?;
    }
    accounts::remove_slot(provider, &id).map_err(|e| e.to_string())?;
    drop(operation);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Project pins (see `pins`)

/// Signs each pinned account's profile in or out (see `accounts::sync_profiles`) and rewrites the
/// launcher. `active` is where accounts are signed in by default, when the caller just read it.
async fn sync_pins(app: &AppHandle, active: Option<accounts::Active>) {
    let state = app.state::<AppState>();
    let pins = state.settings.lock().unwrap().project_pins.clone();
    let wanted = profile_accounts(&state);
    let found = state.sources.lock().unwrap().clone();
    let states = tauri::async_runtime::spawn_blocking(move || {
        // Never decide what is signed in by default from an empty source list.
        let found = if found.is_empty() { sources::discover() } else { found };
        let active = active.unwrap_or_else(|| accounts::capture(&found));
        let states = accounts::sync_profiles(&wanted, &active);
        // The launcher also serves folders without pins (see `launcher`).
        let _ = pins::write_launcher(&pins);
        states
    }).await.unwrap_or_default();
    *state.profile_states.lock().unwrap() = states;
}

/// The accounts whose project profiles stay signed in: pinned ones and leased ones (a leased
/// account's job may run with its profile, see `leases`).
fn profile_accounts(state: &AppState) -> Vec<(Provider, String)> {
    let mut wanted = pins::wanted(&state.settings.lock().unwrap().project_pins);
    for account in state.leases.lock().unwrap().accounts() {
        if !wanted.contains(&account) { wanted.push(account); }
    }
    wanted
}

// ---------------------------------------------------------------------------------------------
// Leases (see `leases`)

/// recommend_account's answer without the accounts leased by anyone but `mine`.
fn recommend_unleased(app: &AppHandle, provider: Provider, expected_percent: Option<f64>, hours: Option<f64>, mine: Option<&str>) -> headroom::Recommendation {
    let state = app.state::<AppState>();
    let snapshot = state.snapshot.lock().unwrap().clone();
    let settings = state.settings.lock().unwrap().clone();
    let rec = headroom::recommend(&snapshot.accounts, &settings, provider, expected_percent, hours, chrono::Utc::now());
    let leased = state.leases.lock().unwrap().others(provider, mine);
    headroom::without_leased(rec, &leased)
}

/// Shows each account's lease on the snapshot (the flyout's "leased: <purpose>").
fn mark_leases(state: &AppState, snapshot: &mut Snapshot) {
    let leases = state.leases.lock().unwrap();
    for a in &mut snapshot.accounts {
        a.leased = leases.holder(a.provider, &a.id).map(|l| l.purpose.clone());
    }
}

/// After leases change: the windows show them and leased accounts' profiles are signed in or out.
async fn leases_changed(app: &AppHandle) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state.snapshot.lock().unwrap().clone();
        mark_leases(&state, &mut snapshot);
        *state.snapshot.lock().unwrap() = snapshot.clone();
        snapshot
    };
    let _ = app.emit("usage-updated", &snapshot);
    crate::alerts::on_snapshot(&app);
    sync_pins(app, None).await;
}

/// Ends expired leases and those whose process exited.
async fn prune_leases(app: &AppHandle) {
    let ended = app.state::<AppState>().leases.lock().unwrap().prune(chrono::Utc::now(), leases::process_alive);
    if ended { leases_changed(app).await; }
}

/// Leases account `id` (else the recommended one among those nobody leased) for `purpose`, and
/// says how to run on it: `config_dir` is its project profile, or null while it is the CLI's
/// current sign-in.
async fn lease_account(app: &AppHandle, provider: Provider, purpose: &str, minutes: Option<u32>, expected_percent: Option<f64>, id: Option<String>, pid: Option<u32>) -> Result<serde_json::Value, String> {
    prune_leases(app).await;
    let state = app.state::<AppState>();
    let (id, label, active) = match id {
        Some(id) => {
            let snapshot = state.snapshot.lock().unwrap();
            let account = snapshot.accounts.iter().find(|a| a.provider == provider && a.id == id).ok_or("monitor_wrong_account")?;
            (id, account.alias.clone().or(account.email.clone()), account.active)
        }
        None => {
            let rec = recommend_unleased(app, provider, expected_percent, None, None);
            let id = rec.recommended.clone().ok_or_else(|| format!("no_account_free: {}", rec.summary))?;
            let active = rec.ranking.iter().any(|c| c.id == id && c.active);
            (id, rec.recommended_label.clone(), active)
        }
    };
    let lease = state.leases.lock().unwrap().take(provider, &id, purpose, minutes, pid, chrono::Utc::now())?;
    leases_changed(app).await;
    let profile = accounts::profiles_root().map(|r| r.join(accounts::profile_name(provider, &id)));
    let ready = state.profile_states.lock().unwrap().get(&(provider, id.clone())) == Some(&accounts::ProfileState::Ready);
    let config_dir = profile.filter(|_| ready && !active).map(|p| p.to_string_lossy().into_owned());
    Ok(serde_json::json!({
        "lease": serde_json::to_value(&lease).map_err(|e| e.to_string())?,
        "id": id, "label": label, "expires_at": lease.expires_at,
        "env_var": pins::env_var(provider), "config_dir": config_dir, "at_default": active,
        "next": "Run the job with env_var set to config_dir for that job only (null: the account is the CLI's current sign-in), then release_lease.",
    }))
}

// ---------------------------------------------------------------------------------------------
// Exact Claude attribution (see `session_hook`)

/// Records which Claude account a session started on: the account signed in to its config dir,
/// or with none given (the default login), the account SlopBar sees signed in to Claude Code.
async fn record_session(app: &AppHandle, session_id: String, cwd: Option<String>, config_dir: Option<String>) -> Result<serde_json::Value, String> {
    if !crate::session_hook::valid_session_id(&session_id) {
        return Err("bad_session_id".into());
    }
    let state = app.state::<AppState>();
    let default = || {
        let snapshot = state.snapshot.lock().unwrap();
        active_log::active_ids(&snapshot.accounts).into_iter().find(|(p, _)| *p == Provider::Claude).map(|(_, id)| id.to_string())
    };
    let id = match config_dir.filter(|d| std::path::Path::new(d).is_absolute()) {
        Some(dir) => tauri::async_runtime::spawn_blocking(move || accounts::login_at(Provider::Claude, std::path::Path::new(&dir))).await.ok().flatten(),
        None => default(),
    };
    let Some(id) = id else { return Ok(serde_json::json!({ "recorded": false, "reason": "unknown_account" })) };
    let now = chrono::Utc::now();
    let changed = state.active_log.lock().unwrap().record_session(&session_id, &id, cwd.as_deref().unwrap_or(""), now);
    if changed {
        let log = state.active_log.lock().unwrap().clone();
        let _ = tauri::async_runtime::spawn_blocking(move || active_log::save(&log)).await;
    }
    Ok(serde_json::json!({ "recorded": true }))
}

/// Claude Code's turn hook (see `turn_hook`, `wind_down`). Only sessions on the default login
/// take part: a switch moves the default login, never a session's own config dir.
async fn record_turn(app: &AppHandle, session_id: String, event: String, config_dir: Option<String>) -> Result<serde_json::Value, String> {
    let event = match event.as_str() {
        "prompt" => wind_down::Event::Prompt,
        "stop" => wind_down::Event::Stop,
        _ => return Err("bad_event".into()),
    };
    if !crate::session_hook::valid_session_id(&session_id) {
        return Err("bad_session_id".into());
    }
    let none = || Ok(serde_json::json!({ "context": null, "block": null }));
    if config_dir.is_some() {
        return none();
    }
    let state = app.state::<AppState>();
    // The account it started on (SessionStart hook); a session SlopBar has not heard of yet is
    // on the default login's account now.
    let known = state.active_log.lock().unwrap().session(&session_id).map(str::to_string);
    let account = match known {
        Some(id) => id,
        None => {
            record_session(app, session_id.clone(), None, None).await?;
            let Some(id) = state.active_log.lock().unwrap().session(&session_id).map(str::to_string) else { return none() };
            id
        }
    };
    let now = chrono::Utc::now();
    let (reply, ready) = {
        let mut tracker = state.wind_down.lock().unwrap();
        let reply = tracker.on_event(&session_id, &account, event, now);
        (reply, event == wind_down::Event::Stop && tracker.ready(now))
    };
    if ready {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { finish_wind_down(&app).await });
    }
    Ok(match reply {
        wind_down::Reply::None => serde_json::json!({ "context": null, "block": null }),
        wind_down::Reply::Context(c) => serde_json::json!({ "context": c, "block": null }),
        wind_down::Reply::Block(b) => serde_json::json!({ "context": null, "block": b }),
    })
}

/// The balancer's switches, with a Claude account that has its own percentage (`rotate_at`) held
/// back while its sessions wind down (see `wind_down`): the wind-down starts at that percentage,
/// and the switch goes ahead once it is ready or usage reaches the hard limit. A wind-down whose
/// reason went away (a manual switch, a reset, no account to move to) ends without switching.
fn graceful_decisions(app: &AppHandle, list: &[model::AccountUsage], settings: &Settings, decisions: Vec<(Provider, String)>) -> Vec<(Provider, String)> {
    let state = app.state::<AppState>();
    let now = chrono::Utc::now();
    let current = list.iter().find(|a| a.provider == Provider::Claude && a.active);
    let own = current.and_then(|a| settings.rotate_at.get(&format!("claude:{}", a.id)).copied());
    let target = decisions.iter().find(|(p, _)| *p == Provider::Claude).map(|(_, id)| id.clone());
    let mut tracker = state.wind_down.lock().unwrap();
    let (Some(target), Some(current), Some(own)) = (target, current, own) else {
        tracker.finish();
        return decisions;
    };
    let peak = current.peak().unwrap_or(0.0);
    let hold = peak < wind_down::hard_limit(own);
    let mut started = None;
    if hold && tracker.active.as_ref().is_none_or(|a| a.from != current.id) {
        tracker.start(&current.id, chrono::Duration::minutes(settings.rotate_grace_minutes as i64), now);
        started = tracker.active.as_ref().map(|a| a.deadline);
    }
    if hold && !tracker.ready(now) {
        drop(tracker);
        if let Some(deadline) = started {
            let label = |id: &str| list.iter().find(|a| a.provider == Provider::Claude && a.id == id).and_then(|a| a.alias.clone().or(a.email.clone()));
            crate::alerts::on_wind_down(app, label(&current.id).as_deref(), label(&target).as_deref(), peak);
            let wait = (deadline - now).to_std().unwrap_or_default() + Duration::from_secs(1);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(wait).await;
                finish_wind_down(&app).await;
            });
        }
        return decisions.into_iter().filter(|(p, _)| *p != Provider::Claude).collect();
    }
    tracker.finish();
    decisions
}

/// Ends a wind-down that is ready: switches to the account the balancer picks now (the one chosen
/// at the start may have changed), unless the account is no longer signed in or no longer needs it.
async fn finish_wind_down(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(done) = ({
        let mut tracker = state.wind_down.lock().unwrap();
        if tracker.ready(chrono::Utc::now()) { tracker.finish() } else { None }
    }) else { return };
    let (accounts, settings) = (state.snapshot.lock().unwrap().accounts.clone(), state.settings.lock().unwrap().clone());
    if !accounts.iter().any(|a| a.provider == Provider::Claude && a.active && a.id == done.from) {
        return;
    }
    let leased = state.leases.lock().unwrap().accounts();
    let Some((_, target)) = balancer::decide_avoiding(&accounts, &settings, &HashMap::new(), &leased)
        .into_iter().find(|(p, _)| *p == Provider::Claude) else { return };
    if switch_to(app, Provider::Claude, target, true).await.is_ok() {
        state.last_switch.lock().unwrap().insert(Provider::Claude, Instant::now());
        do_refresh(app).await;
    }
}

fn pin_listing(app: &AppHandle) -> pins::Listing {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let snapshot = state.snapshot.lock().unwrap().clone();
    let states = state.profile_states.lock().unwrap().clone();
    let label = |provider: Provider, id: &str| snapshot.accounts.iter()
        .find(|a| a.provider == provider && a.id == id)
        .and_then(|a| a.alias.clone().or(a.email.clone()));
    let root = accounts::profiles_root().unwrap_or_default();
    pins::listing(&settings.project_pins, &states, label, &root, pins::launcher_path().as_deref())
}

#[tauri::command]
fn list_project_pins(app: AppHandle) -> pins::Listing {
    pin_listing(&app)
}

#[tauri::command]
async fn set_project_pin(app: AppHandle, folder: String, provider: Provider, id: Option<String>) -> Result<pins::Listing, String> {
    {
        let state = app.state::<AppState>();
        let _held = state.settings_lock.lock().await;
        let current = state.settings.lock().unwrap().clone();
        let known: Vec<_> = accounts::load_slots().into_iter().map(|s| (s.provider, s.id)).collect();
        let project_pins = pins::set(&current.project_pins, &folder, provider, id.as_deref(), &known)?;
        store_settings(&app, Settings { project_pins, ..current }).await?;
    }
    sync_pins(&app, None).await;
    Ok(pin_listing(&app))
}

// ---------------------------------------------------------------------------------------------
// The quick switcher

/// How long the switcher stays loaded after it hides, so opening it again soon is instant.
const SWITCHER_LINGER: Duration = Duration::from_secs(30);
/// How long after a blur the switcher checks whether it really lost the foreground.
const SWITCHER_BLUR_SETTLE: Duration = Duration::from_millis(80);

/// Settings and the quick switcher are not created at startup (`"create": false`): each is
/// built when first opened and destroyed when closed, so neither keeps a WebView2 renderer
/// while unused. Whatever must outlive them is kept here or in settings, not in the page. The
/// web taskbar widget is built only while the setting `native_widget` is off (`dock::apply_mode`),
/// and the flyout hidden shortly after startup, kept for instant opening (see `panel::prepare`).
pub(crate) const ON_DEMAND: [&str; 4] = [SETTINGS_LABEL, SWITCHER_LABEL, dock::LABEL, panel::LABEL];

/// The window `label` (one of `ON_DEMAND`), created from its configuration if it is not open.
/// Builds off the main thread: building a window there waits on the event loop it runs.
pub(crate) async fn on_demand_window(app: &AppHandle, label: &str) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    let config = app.config().app.windows.iter().find(|w| w.label == label)?.clone();
    let h = app.clone();
    let built = tauri::async_runtime::spawn_blocking(move || tauri::WebviewWindowBuilder::from_config(&h, &config).ok()?.build().ok())
        .await.ok().flatten();
    // Two callers raced: the other one built it.
    let Some(win) = built else { return app.get_webview_window(label) };
    match label {
        SETTINGS_LABEL => watch_settings(&win),
        SWITCHER_LABEL => watch_switcher(&win),
        #[cfg(windows)]
        dock::LABEL => dock::exclude_from_peek(&win),
        panel::LABEL => panel::watch(app, &win),
        _ => {}
    }
    Some(win)
}

/// Closing Settings destroys it (the default); only its Mica backdrop is set here.
fn watch_settings(win: &tauri::WebviewWindow) {
    if dock::is_windows_11() {
        use tauri::window::{Effect, EffectsBuilder};
        let _ = win.set_effects(EffectsBuilder::new().effect(Effect::Mica).build());
    }
}

/// Like a launcher: clicking anywhere else hides it, and it is destroyed once it has stayed
/// hidden for `SWITCHER_LINGER`. Closing it (Alt+F4) destroys it at once.
fn watch_switcher(win: &tauri::WebviewWindow) {
    #[cfg(windows)]
    dock::exclude_from_peek(win);
    let w = win.clone();
    win.on_window_event(move |e| {
        if let WindowEvent::Focused(false) = e {
            // Just shown, WebView2 takes the keyboard focus from the window, which reports a blur
            // while the switcher is still in front; hiding then made it flash and vanish. Hide only
            // once the foreground has really gone elsewhere.
            let w = w.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(SWITCHER_BLUR_SETTLE).await;
                if w.is_visible().unwrap_or(false) && !dock::is_foreground(&w) {
                    hide_switcher_window(&w);
                }
            });
        }
    });
}

/// Hides the switcher, then destroys it unless it was shown again in the meantime.
fn hide_switcher_window(w: &tauri::WebviewWindow) {
    let _ = w.hide();
    let w = w.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SWITCHER_LINGER).await;
        if !w.is_visible().unwrap_or(true) {
            let _ = w.destroy();
        }
    });
}

fn show_switcher(app: &AppHandle, w: &tauri::WebviewWindow) {
    let _ = w.center();
    let _ = w.show();
    let _ = w.set_focus();
    let _ = app.emit_to(SWITCHER_LABEL, "switcher-open", ());
}

/// Shows the switcher in the middle of the screen, or hides it when it is open (the shortcut).
pub(crate) fn toggle_switcher(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(w) = on_demand_window(&app, SWITCHER_LABEL).await else { return };
        if w.is_visible().unwrap_or(false) { hide_switcher_window(&w); } else { show_switcher(&app, &w); }
    });
}

#[tauri::command]
async fn open_switcher(app: AppHandle) {
    // The switcher takes the focus while the flyout still holds the foreground; the flyout then
    // hides on its own blur. Hiding the flyout first handed the foreground back to the previous
    // app while the switcher was being built, and the switcher hid as soon as it appeared.
    if let Some(w) = on_demand_window(&app, SWITCHER_LABEL).await {
        if !w.is_visible().unwrap_or(false) { show_switcher(&app, &w); }
    }
    if let Some(panel) = app.get_webview_window(panel::LABEL) {
        if panel.is_visible().unwrap_or(false) { panel::close(&app); }
    }
}

#[tauri::command]
fn hide_switcher(app: AppHandle) {
    if let Some(w) = app.get_webview_window(SWITCHER_LABEL) {
        hide_switcher_window(&w);
    }
}

/// A settings shortcut ("Ctrl+Alt+S") as the plugin parses it.
fn parse_hotkey(hotkey: &str) -> Option<tauri_plugin_global_shortcut::Shortcut> {
    hotkey.replace("Win", "Super").parse().ok()
}

/// Registers the system-wide shortcuts from settings (the switcher's and the compact toggle's),
/// replacing the previous ones.
fn apply_hotkey(app: &AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let (switcher, compact) = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        (s.switcher_hotkey.clone(), s.compact_hotkey.clone())
    };
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    let register = |hotkey: &str| if hotkey.is_empty() {
        None
    } else {
        match parse_hotkey(hotkey) {
            Some(shortcut) => shortcuts.register(shortcut).err().map(|_| "hotkey_taken".to_string()),
            None => Some("hotkey_invalid".to_string()),
        }
    };
    let state = app.state::<AppState>();
    *state.hotkey_error.lock().unwrap() = register(&switcher);
    *state.compact_hotkey_error.lock().unwrap() = register(&compact);
}

/// A pressed shortcut: the compact toggle's, or else the switcher's.
fn hotkey_pressed(app: &AppHandle, shortcut: &tauri_plugin_global_shortcut::Shortcut) {
    let compact = app.state::<AppState>().settings.lock().unwrap().compact_hotkey.clone();
    if !compact.is_empty() && parse_hotkey(&compact).as_ref() == Some(shortcut) {
        toggle_compact_flyout(app, true);
    } else {
        toggle_switcher(app);
    }
}

/// Why a shortcut is not working, if it is not: "hotkey_taken" or "hotkey_invalid". `which` is
/// "compact" for the compact toggle's; otherwise the switcher's.
#[tauri::command]
fn hotkey_status(state: tauri::State<AppState>, which: Option<String>) -> Option<String> {
    let error = if which.as_deref() == Some("compact") { &state.compact_hotkey_error } else { &state.hotkey_error };
    error.lock().unwrap().clone()
}

/// The flyout's own layout button: the flyout redraws in the new layout, so no toast.
#[tauri::command]
fn toggle_compact(app: AppHandle) {
    toggle_compact_flyout(&app, false);
}

/// Turns the compact flyout on or off. From the menus and the shortcut it is `announce`d in the
/// widget, since the flyout is usually closed.
pub(crate) fn toggle_compact_flyout(app: &AppHandle, announce: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let _held = state.settings_lock.lock().await;
        let mut settings = state.settings.lock().unwrap().clone();
        settings.compact_flyout = !settings.compact_flyout;
        // Choosing a layout answers the flyout's offer of the compact one.
        settings.compact_suggested = true;
        let on = settings.compact_flyout;
        if store_settings(&app, settings).await.is_ok() && announce {
            let _ = app.emit("compact-toggled", on);
        }
    });
}

#[derive(Serialize)]
struct Disconnected { snapshot: Snapshot, revoked: bool }

#[tauri::command]
async fn disconnect_monitoring(app: AppHandle, provider: Provider, id: String) -> Result<Disconnected, String> {
    let revoked = disconnect_one(&app, provider, &id).await?;
    Ok(Disconnected { snapshot: do_refresh(&app).await, revoked })
}

/// What "Revoke all SlopBar sessions" did for one account.
#[derive(Serialize)]
struct RevokedSessions {
    snapshot: Snapshot,
    /// "revoked" (the provider confirmed), "unconfirmed" (forgotten here, the provider did not
    /// confirm) or "none" (no monitoring sign-in was connected).
    monitoring: &'static str,
    slot_removed: bool,
    /// Always "managed_by_cli": the CLI's own sign-in is the CLI's to end (its logout).
    cli_login: &'static str,
}

/// Whether the account has monitoring connected, and whether it is in use.
fn session_facts(app: &AppHandle, provider: Provider, id: &str) -> Option<(bool, bool)> {
    let snapshot = app.state::<AppState>().snapshot.lock().unwrap().clone();
    snapshot.accounts.iter().find(|a| a.provider == provider && a.id == id).map(|a| (a.monitor_connected, a.active))
}

/// Revokes every sign-in SlopBar itself holds for an account: its monitoring session, at the
/// provider and here. The CLI's own login stays the CLI's; with `remove_slot` the saved copy of it
/// in the vault goes too (as Remove does), which needs the account not to be in use.
async fn revoke_sessions(app: &AppHandle, provider: Provider, id: &str, remove_slot: bool) -> Result<RevokedSessions, String> {
    let (connected, active) = session_facts(app, provider, id).ok_or("monitor_wrong_account")?;
    if remove_slot && active {
        return Err("active".into());
    }
    let monitoring = match (connected, disconnect_one(app, provider, id).await?) {
        (_, true) => "revoked",
        (true, false) => "unconfirmed",
        (false, false) => "none",
    };
    if remove_slot {
        remove_one(app, provider, id).await?;
    }
    Ok(RevokedSessions { snapshot: do_refresh(app).await, monitoring, slot_removed: remove_slot, cli_login: "managed_by_cli" })
}

#[tauri::command]
async fn revoke_account_sessions(app: AppHandle, provider: Provider, id: String, remove_slot: bool) -> Result<RevokedSessions, String> {
    revoke_sessions(&app, provider, &id, remove_slot).await
}

/// Revokes and forgets the account's monitoring sign-in; whether the provider confirmed it.
async fn disconnect_one(app: &AppHandle, provider: Provider, id: &str) -> Result<bool, String> {
    let id = id.to_string();
    let operation = app.state::<AppState>().account_operations.begin(provider, &id)?;
    let slot = accounts::load_slots().into_iter().find(|s| s.provider == provider && s.id == id)
        .ok_or("monitor_wrong_account")?;
    let started = Instant::now();
    let result = app.state::<AppState>().monitor.disconnect(&slot, &http_client()).await;
    app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::Disconnect,
        code: diagnostics::Code::from_error(result.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
    let revoked = result?;
    auto_monitor_saved(app, |t| t.manual(&monitoring::key(&slot), false));
    {
        let state = app.state::<AppState>();
        let _refresh = state.refresh_lock.lock().await;
        providers::retry_now(&mut state.usage_cache.lock().unwrap(), &monitoring::key(&slot));
    }
    drop(operation);
    Ok(revoked)
}

#[tauri::command]
async fn check_for_updates(app: AppHandle) -> Result<updates::UpdateInfo, String> {
    updates::check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    updates::install(&app).await
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    let version = app.package_info().version.to_string();
    if cfg!(feature = "upstream-updates") { version } else { format!("{version} (independent monitoring)") }
}

#[tauri::command]
fn updates_enabled() -> bool { cfg!(feature = "upstream-updates") }

/// The OS display language (WebView2's navigator.languages doesn't follow it reliably).
#[tauri::command]
fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}

/// Sync folders (OneDrive, Dropbox, iCloud Drive) Settings can offer for sharing in-use status.
#[tauri::command]
async fn sync_folders() -> Vec<presence::SyncFolder> {
    tauri::async_runtime::spawn_blocking(presence::sync_folders).await.unwrap_or_default()
}

#[derive(Serialize)]
struct RcloneRemotes { installed: bool, remotes: Vec<crate::rclone::Remote> }

/// The user's rclone remotes (names and types only), for sharing in-use status without a sync app.
#[tauri::command]
async fn rclone_remotes() -> RcloneRemotes {
    tauri::async_runtime::spawn_blocking(|| match crate::rclone::find() {
        Some(exe) => RcloneRemotes { installed: true, remotes: crate::rclone::remotes(&crate::rclone::Cli(exe)) },
        None => RcloneRemotes { installed: false, remotes: vec![] },
    }).await.unwrap_or(RcloneRemotes { installed: false, remotes: vec![] })
}

/// Whether a typed folder can hold presence files.
#[tauri::command]
fn check_presence_folder(folder: String) -> bool {
    settings::valid_presence_folder(folder.trim())
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _held = state.settings_lock.lock().await;
    store_settings(&app, settings).await
}

/// Saves and applies settings; callers hold `settings_lock`.
async fn store_settings(app: &AppHandle, settings: Settings) -> Result<(), String> {
    // A changed shortcut is checked here too: the window saves whole settings.
    let (stored, stored_compact) = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        (s.switcher_hotkey.clone(), s.compact_hotkey.clone())
    };
    let settings = if settings.switcher_hotkey == stored { settings } else {
        Settings { switcher_hotkey: settings::normalize_hotkey(&settings.switcher_hotkey)?, ..settings }
    };
    let settings = if settings.compact_hotkey == stored_compact { settings } else {
        Settings { compact_hotkey: settings::normalize_hotkey(&settings.compact_hotkey)?, ..settings }
    };
    settings::distinct_hotkeys(&settings)?;
    // And a changed phone-alert URL: only an ntfy topic URL, never plain http to the internet.
    let stored_phone = app.state::<AppState>().settings.lock().unwrap().phone.url.clone();
    let mut settings = settings;
    if settings.phone.url != stored_phone {
        if let Some(url) = settings.phone.url.as_deref().filter(|u| !u.trim().is_empty()) {
            settings.phone.url = Some(phone::normalize_url(url)?);
        } else {
            settings.phone.url = None;
        }
    }
    if settings.phone.url.is_none() { settings.phone.enabled = false; }
    settings::save(&settings)?;
    diagnostics::set_crash_records(settings.crash_records);
    let previous = std::mem::replace(&mut *app.state::<AppState>().settings.lock().unwrap(), settings.clone());
    apply_settings(app);
    let providers_changed = previous.providers.claude != settings.providers.claude
        || previous.providers.codex != settings.providers.codex;
    let presence_changed = previous.presence_folder != settings.presence_folder || previous.presence_remote != settings.presence_remote;
    if previous.switcher_hotkey != settings.switcher_hotkey || previous.compact_hotkey != settings.compact_hotkey {
        apply_hotkey(app);
    }
    if presence_changed {
        app.state::<AppState>().health.lock().unwrap().sharing = None;
        // The old folder's other machines should not keep seeing this one as signed in.
        if let Some(old) = presence::store(&previous) {
            let interval = settings.refresh_minutes.max(1) as u64 * 60;
            let _ = tauri::async_runtime::spawn_blocking(move || presence::withdraw(old.as_ref(), interval)).await;
        }
    }
    if presence_changed || previous.account_roster != settings.account_roster {
        // Off, or moved: the old location's machines should not keep this machine's accounts.
        if previous.account_roster {
            if let Some(old) = roster::store(&previous) {
                let _ = tauri::async_runtime::spawn_blocking(move || roster::withdraw(old.as_ref())).await;
            }
        }
        let state = app.state::<AppState>();
        state.roster.lock().unwrap().reset();
        *state.roster_seen.lock().unwrap() = roster::Seen::default();
    }
    if providers_changed || presence_changed || (settings.account_roster && !previous.account_roster) {
        do_refresh(app).await;
    }
    Ok(())
}

/// Shows Settings, on `page` when given (the flyout's health badge opens Health).
#[tauri::command]
fn open_settings(app: AppHandle, page: Option<String>) {
    show_settings_page(&app, page);
}

/// The page Settings was asked to open on while it was being created, once.
#[tauri::command]
fn take_settings_page(state: tauri::State<AppState>) -> Option<String> {
    state.settings_page.lock().unwrap().take()
}

#[tauri::command]
fn set_panel_height(app: AppHandle, height: f64) {
    *app.state::<AppState>().panel_height.lock().unwrap() = height;
    panel::page_ready();
    panel::place(&app);
}

#[tauri::command]
fn set_dock_width(app: AppHandle, width: f64) {
    *app.state::<AppState>().dock_width.lock().unwrap() = width;
}

/// A click on the taskbar widget (web or native) opens the flyout above it.
#[tauri::command]
pub(crate) fn dock_clicked(app: AppHandle) {
    if let Some(anchor) = dock::anchor(&app) {
        *app.state::<AppState>().panel_anchor.lock().unwrap() = Some(anchor);
    }
    panel::toggle(&app);
}

/// The Windows accent color as WinUI uses it on light and dark backgrounds
/// (SystemAccentColorDark1 and SystemAccentColorLight2), from the system's accent palette.
#[tauri::command]
fn accent_colors() -> Option<(String, String)> {
    let p = dock::accent_palette()?;
    let hex = |i: usize| format!("#{:02x}{:02x}{:02x}", p[i * 4], p[i * 4 + 1], p[i * 4 + 2]);
    // Palette entries: Light3, Light2, Light1, Accent, Dark1, Dark2, Dark3, (unused).
    Some((hex(4), hex(1)))
}

/// Whether the settings window is on Mica (Windows 11); elsewhere it paints its own background.
#[tauri::command]
fn has_mica() -> bool {
    dock::is_windows_11()
}

#[tauri::command]
fn taskbar_theme() -> &'static str {
    if dock::taskbar_is_light() { "light" } else { "dark" }
}

/// Right-click on the widget: the native context menu, like the built-in tray items have.
#[tauri::command]
fn dock_menu(app: AppHandle) {
    let menu = app.state::<AppState>().menu.lock().unwrap().clone();
    if let (Some(menu), Some(w)) = (menu, app.get_webview_window(dock::LABEL)) {
        let _ = w.popup_menu(&menu);
    }
}

/// Menu labels come from the frontend, which owns the translations.
#[tauri::command]
fn set_menu_labels(state: tauri::State<AppState>, labels: HashMap<String, String>) {
    for item in state.menu_items.lock().unwrap().iter() {
        if let Some(text) = labels.get(item.id().as_ref()) {
            let _ = item.set_text(text);
        }
    }
    if let (Some(item), Some(text)) = (state.compact_item.lock().unwrap().as_ref(), labels.get(MENU_COMPACT)) {
        let _ = item.set_text(text);
    }
}

pub(crate) fn show_settings(app: &AppHandle) {
    show_settings_page(app, None);
}

/// Shows Settings, on `page` if it is one of its pages, creating the window if it is closed.
pub(crate) fn show_settings_page(app: &AppHandle, page: Option<String>) {
    let page = page.filter(|p| door::SETTINGS_PAGES.contains(&p.as_str()));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let w = match app.get_webview_window(SETTINGS_LABEL) {
            Some(w) => {
                if let Some(page) = &page {
                    let _ = app.emit_to(SETTINGS_LABEL, "show-page", page);
                }
                w
            }
            None => {
                // The new page reads it when it loads (`take_settings_page`).
                *app.state::<AppState>().settings_page.lock().unwrap() = page;
                let Some(w) = on_demand_window(&app, SETTINGS_LABEL).await else { return };
                w
            }
        };
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    });
}

/// The widget's right-click menu (the web widget's Tauri menu, or the native widget's own).
pub(crate) fn menu_event(app: &AppHandle, id: &str) {
    match id {
        "refresh" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { do_refresh(&app).await });
        }
        "settings" => show_settings(app),
        MENU_COMPACT => toggle_compact_flyout(app, true),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Applies the settings that live outside the webviews.
pub(crate) fn apply_settings(app: &AppHandle) {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let autolaunch = app.autolaunch();
    if !settings::isolated() && autolaunch.is_enabled().unwrap_or(false) != settings.launch_at_login {
        let _ = if settings.launch_at_login { autolaunch.enable() } else { autolaunch.disable() };
    }
    dock::apply_mode(app, settings.native_widget);
    sources::set_wsl_mode(settings.wsl_accounts);
    if let Some(item) = app.state::<AppState>().compact_item.lock().unwrap().as_ref() {
        let _ = item.set_checked(settings.compact_flyout);
    }
    let _ = app.emit("settings-changed", &settings);
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("http client")
}

/// Captures live logins into the vault, fetches every account's usage, then lets the balancer
/// switch accounts that reached their threshold. The widget gets the new numbers as soon as they
/// are known; sharing and bookkeeping follow, and the index, calendar and Codex scans run after.
async fn do_refresh(app: &AppHandle) -> Snapshot {
    use diagnostics::RefreshPhase as P;
    let state = app.state::<AppState>();
    let _refresh = state.refresh_lock.lock().await;
    // Ended leases free their accounts (and profiles) before this refresh decides anything.
    state.leases.lock().unwrap().prune(chrono::Utc::now(), leases::process_alive);
    let started = Instant::now();
    let mut phases = diagnostics::Phases::start();
    let settings = state.settings.lock().unwrap().clone();
    state.diagnostics.poll_started(settings.refresh_minutes);
    let found = tauri::async_runtime::spawn_blocking(sources::discover)
        .await
        .unwrap_or_default();
    *state.sources.lock().unwrap() = found.clone();
    phases.end(P::Discover);
    let profiles = accounts::profile_sources(&profile_accounts(&state));
    accounts::resolve_identities(&http_client(), &[found.clone(), profiles].concat()).await;
    phases.end(P::Identities);
    let scan = found.clone();
    let active = tauri::async_runtime::spawn_blocking(move || accounts::capture(&scan))
        .await
        .unwrap_or_default();
    phases.end(P::Capture);
    // Before reading the vault: renewals made in project profiles are synced into it.
    sync_pins(app, Some(active.clone())).await;
    phases.end(P::Pins);
    let slots = tauri::async_runtime::spawn_blocking(accounts::load_slots).await.unwrap_or_default();
    phases.end(P::Vault);
    static CLEANUP_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !CLEANUP_STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        // Leftover sign-in folders from earlier runs: once, beside this refresh.
        let (app, homes, slots) = (app.clone(), found.clone(), slots.clone());
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let protected = state.monitor.protected_tokens(&slots).await;
            let started = Instant::now();
            let result = tauri::async_runtime::spawn_blocking(move || crate::login::cleanup(&homes, protected.as_ref())).await;
            state.diagnostics.record(None, None, diagnostics::Event::Action { action: diagnostics::Action::Cleanup,
                code: if result.is_ok() { diagnostics::Code::Ok } else { diagnostics::Code::Other }, elapsed_ms: diagnostics::millis(started) });
        });
    }

    let mut cache = state.usage_cache.lock().unwrap().clone();
    let mut list = providers::fetch_all(&http_client(), &slots, &active, &mut cache, &settings, &state.monitor).await;
    let valid: HashSet<_> = slots.iter().map(monitoring::key).collect();
    cache.retain(|key, _| valid.contains(key));
    *state.usage_cache.lock().unwrap() = cache.clone();
    phases.end(P::Fetch);

    let decisions = {
        let last = state.last_switch.lock().unwrap().clone();
        let leased = state.leases.lock().unwrap().accounts();
        balancer::decide_avoiding(&list, &settings, &last, &leased)
    };
    let decisions = graceful_decisions(app, &list, &settings, decisions);
    for (provider, id) in decisions {
        // Publish the current numbers first so the switch notice can name both accounts.
        *state.snapshot.lock().unwrap() = Snapshot { accounts: list.clone(), updated_at: Some(chrono::Utc::now()) };
        if switch_to(app, provider, id.clone(), true).await.is_ok() {
            state.last_switch.lock().unwrap().insert(provider, Instant::now());
            for a in list.iter_mut().filter(|a| a.provider == provider) {
                let now_active = a.id == id;
                if now_active != a.active {
                    a.sources = if now_active {
                        found.iter().map(|s| s.label.clone()).collect()
                    } else {
                        vec![]
                    };
                    a.active = now_active;
                }
            }
        }
    }
    phases.end(P::Balance);

    // The widget redraws now: what sharing and forecasts add is carried over from the last
    // refresh until they are worked out below.
    {
        let mut early = list.clone();
        carry_over(&mut early, &state.snapshot.lock().unwrap().accounts);
        let early = Snapshot { accounts: early, updated_at: Some(chrono::Utc::now()) };
        *state.snapshot.lock().unwrap() = early.clone();
        let _ = app.emit("usage-updated", &early);
        crate::alerts::on_snapshot(&app);
    }
    phases.end(P::Publish);
    if providers::save_cache(&cache).is_err() {
        state.diagnostics.record(None, None, diagnostics::Event::Action { action: diagnostics::Action::CacheSave, code: diagnostics::Code::Storage, elapsed_ms: 0 });
    }

    // Other machines: what they share through the presence folder, then what rising usage implies.
    let now = chrono::Utc::now();
    let interval = settings.refresh_minutes.max(1) as u64 * 60;
    // Whether every computer is covered: sharing is on and someone (this one or a peer) published.
    let mut covered = false;
    let mut shared = None;
    if let Some(store) = presence::store(&settings) {
        let mine = list.clone();
        let (peers, wrote, read) = tauri::async_runtime::spawn_blocking(move || {
            let Some((id, name)) = presence::identity() else { return (vec![], false, false) };
            let wrote = store.write(&presence::own(id, name, interval, &mine, now)).is_ok();
            let listed = store.list();
            let read = listed.is_ok();
            (presence::peers(listed.unwrap_or_default(), id, now), wrote, read)
        }).await.unwrap_or_default();
        shared = Some(health::SharingResult { at: now, wrote, read });
        covered = wrote || peers.iter().any(|p| p.live);
        presence::attach(&mut list, &peers, now);
    }
    // Two refresh intervals without a rise (readings are at least a minute apart), plus slack.
    let hold = chrono::Duration::seconds(2 * interval.max(60) as i64 + 30);
    // Consecutive readings of a few refreshes at most (a missed one is fine; a night asleep is not).
    let surge = presence::Surge {
        min_rise: settings.unexpected_min_rise.max(1) as f64,
        max_gap: chrono::Duration::seconds((3 * interval as i64 + 60).max(15 * 60)),
    };
    let surged = presence::infer(&mut state.presence_tracks.lock().unwrap(), &mut list, hold, surge, now);
    if settings.account_roster {
        sync_roster(&state, &settings, list.clone(), false, false).await;
    }
    let watch = unexpected::Watch { covered, notify: settings.notifications.unexpected_use, web_use: &settings.web_use };
    let log = {
        let mut log = state.unexpected_log.lock().unwrap();
        unexpected::apply(&mut log, &mut list, &surged, &watch, now).then(|| log.clone())
    };
    if let Some(log) = log {
        let _ = unexpected::save(&log);
    }
    phases.end(P::Presence);

    let mut snapshot = Snapshot { accounts: list, updated_at: Some(chrono::Utc::now()) };
    state.diagnostics.samples(&snapshot);
    note_active(&state, &snapshot, settings.refresh_minutes);
    let history = {
        let mut history = state.usage_history.lock().unwrap();
        subscriptions::record(&mut history, &snapshot, &valid, chrono::Utc::now());
        history.clone()
    };
    {
        let mut recent = state.recent.lock().unwrap();
        let now = chrono::Utc::now();
        recent.record(&snapshot.accounts, now);
        headroom::attach_forecasts(&mut snapshot, &settings, &recent, now);
    }
    if subscriptions::save(&history).is_err() { state.diagnostics.record(None, None, diagnostics::Event::Action { action: diagnostics::Action::HistorySave, code: diagnostics::Code::Storage, elapsed_ms: 0 }); }
    state.diagnostics.record(None, None, diagnostics::Event::PollFinished { elapsed_ms: diagnostics::millis(started), accounts: snapshot.accounts.len(),
        native_sources: found.iter().filter(|s| s.distro.is_none()).count(), wsl_sources: found.iter().filter(|s| s.distro.is_some()).count() });
    {
        let mut seen = state.health.lock().unwrap();
        seen.sharing = shared;
        if snapshot.accounts.is_empty() || snapshot.accounts.iter().any(|a| !a.stale) {
            seen.last_success = snapshot.updated_at;
        }
    }
    mark_leases(&state, &mut snapshot);
    phases.end(P::Bookkeeping);
    *state.snapshot.lock().unwrap() = snapshot.clone();
    record_spend(&state);
    let _ = app.emit("usage-updated", &snapshot);
    crate::alerts::on_snapshot(&app);
    phases.end(P::Final);
    state.diagnostics.record(None, None, phases.event());
    // Off the refresh: each runs one at a time on its own.
    if settings.providers.codex { scan_codex_local(app); }
    if settings.calendar.enabled {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { let _ = write_calendar(&app).await; });
    }
    scan_usage_index(app);
    snapshot
}

/// Gives freshly fetched accounts what the last refresh worked out from sharing and forecasts,
/// so the early redraw does not blank them for a moment.
fn carry_over(list: &mut [crate::model::AccountUsage], previous: &[crate::model::AccountUsage]) {
    for a in list.iter_mut() {
        if let Some(p) = previous.iter().find(|p| p.provider == a.provider && p.id == a.id) {
            a.remote = p.remote.clone();
            a.used_outside = p.used_outside;
            a.runs_out_at = p.runs_out_at;
            a.unexpected = p.unexpected.clone();
        }
    }
}

/// Records which account each CLI has signed in, seeding the log once from the diagnostics
/// journal's past week. Saved when an interval starts, else every ten minutes.
fn note_active(state: &AppState, snapshot: &Snapshot, refresh_minutes: u32) {
    let now = chrono::Utc::now();
    let gap = active_log::max_gap(refresh_minutes);
    let marks = (!state.active_log.lock().unwrap().backfilled).then(|| state.diagnostics.active_marks());
    let mut log = state.active_log.lock().unwrap();
    let mut started = false;
    if let Some(marks) = marks {
        // The journal keeps a steady reading only hourly.
        log.backfill(marks, chrono::Duration::minutes(75));
        started = true;
    }
    for (provider, id) in active_log::active_ids(&snapshot.accounts) {
        started |= log.note(provider, id, now, gap);
    }
    let mut saved = state.active_saved.lock().unwrap();
    if started || saved.map_or(true, |t| t.elapsed() >= Duration::from_secs(600)) {
        log.prune(now);
        let _ = active_log::save(&log);
        *saved = Some(Instant::now());
    }
}

/// Re-reads the CLIs' transcripts and rollouts off the async runtime, one scan at a time. The
/// first run reads weeks of files; later ones only what changed.
fn scan_usage_index(app: &AppHandle) {
    static SCANNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SCANNING.swap(true, std::sync::atomic::Ordering::AcqRel) { return; }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let index = tauri::async_runtime::spawn_blocking(|| usage_index::scan(chrono::Utc::now())).await;
        SCANNING.store(false, std::sync::atomic::Ordering::Release);
        let Ok(index) = index else { return };
        *app.state::<AppState>().usage_index.lock().unwrap() = Some(index.clone());
        let _ = tauri::async_runtime::spawn_blocking(move || check_burn(&app, &index)).await;
    });
}

/// Looks for runaway agents (see `burn`) with the index just read, queuing their notifications
/// for the taskbar widget.
fn check_burn(app: &AppHandle, index: &usage_index::Index) {
    let state = app.state::<AppState>();
    let snapshot = state.snapshot.lock().unwrap().clone();
    if snapshot.updated_at.is_none() { return; }
    let settings = state.settings.lock().unwrap().clone();
    let now = chrono::Utc::now();
    let log = state.active_log.lock().unwrap().clone();
    let window = chrono::Duration::minutes(settings.burn.minutes as i64);
    let rates = usage_index::local_rates(index, &log, &usage_index::Known::from_snapshot(&snapshot), window, chrono::Duration::days(burn::BASELINE_DAYS), now);
    let found = burn::detect(&snapshot.accounts, &state.recent.lock().unwrap(), &rates, &settings, now);
    let (due, changed) = {
        let mut log = state.burn_log.lock().unwrap();
        let (due, changed) = burn::apply(&mut log, found, settings.notifications.burn, now);
        (due, changed.then(|| log.clone()))
    };
    if let Some(log) = changed { let _ = burn::save(&log); }
    state.burn_due.lock().unwrap().extend(due);
}

/// The last index, or a scan now when there is none yet.
async fn usage_index_now(app: &AppHandle) -> std::sync::Arc<usage_index::Index> {
    if let Some(index) = app.state::<AppState>().usage_index.lock().unwrap().clone() { return index; }
    let index = tauri::async_runtime::spawn_blocking(|| usage_index::scan(chrono::Utc::now())).await
        .unwrap_or_else(|_| std::sync::Arc::new(usage_index::Index::default()));
    *app.state::<AppState>().usage_index.lock().unwrap() = Some(index.clone());
    index
}

async fn usage_breakdown(app: &AppHandle, filter: usage_index::Filter) -> usage_index::Breakdown {
    let index = usage_index_now(app).await;
    let state = app.state::<AppState>();
    let known = usage_index::Known::from_snapshot(&state.snapshot.lock().unwrap());
    let log = state.active_log.lock().unwrap().clone();
    usage_index::breakdown(&index, &log, &known, &filter, chrono::Utc::now(), &chrono::Local)
}

#[tauri::command]
async fn get_usage_breakdown(app: AppHandle, provider: Option<Provider>, account: Option<String>, days: Option<u32>, group_by: Option<usage_index::GroupBy>) -> usage_index::Breakdown {
    usage_breakdown(&app, usage_index::Filter { provider, account, days: days.unwrap_or(7), group_by: group_by.unwrap_or_default() }).await
}

/// Writes the CSV exports into `folder` (default Documents\SlopBar Exports).
async fn write_csv(app: &AppHandle, folder: Option<String>) -> Result<csv_export::Written, String> {
    let folder = match folder.map(|f| f.trim().to_string()).filter(|f| !f.is_empty()) {
        Some(f) => std::path::PathBuf::from(f),
        None => csv_export::default_folder().ok_or("no_documents_folder")?,
    };
    let index = usage_index_now(app).await;
    let state = app.state::<AppState>();
    let snapshot = state.snapshot.lock().unwrap().clone();
    let known = usage_index::Known::from_snapshot(&snapshot);
    let log = state.active_log.lock().unwrap().clone();
    let history = state.usage_history.lock().unwrap().clone();
    let report = subscription_report(&state);
    let ledger = state.ledger.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let now = chrono::Utc::now();
        let rows = usage_index::daily(&index, &log, &known, now, &chrono::Local);
        let stamp = now.with_timezone(&chrono::Local).format("%Y-%m-%d-%H%M%S").to_string();
        csv_export::write(&folder, &csv_export::history_csv(&history, &known), &csv_export::breakdown_csv(&rows), &csv_export::subscriptions_csv(&report), &csv_export::ledger_csv(&ledger), &stamp)
    }).await.map_err(|_| "export_folder_unwritable".to_string())?
}

#[tauri::command]
async fn export_csv(app: AppHandle, folder: Option<String>) -> Result<csv_export::Written, String> {
    let written = write_csv(&app, folder).await?;
    #[cfg(windows)]
    {
        let folder = written.folder.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || std::process::Command::new("explorer.exe").arg(folder).spawn()).await;
    }
    Ok(written)
}

#[tauri::command]
fn csv_default_folder() -> Option<String> {
    csv_export::default_folder().map(|p| p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------------------------
// Backup and restore (see `backup`)

/// Seals settings, the weekly history and the spend ledger into a backup file. Only the slots'
/// provider, id and email are read from the vault.
async fn write_backup(app: &AppHandle, path: Option<String>, passphrase: backup::Passphrase) -> Result<serde_json::Value, String> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let history = state.usage_history.lock().unwrap().clone();
    let ledger = state.ledger.lock().unwrap().clone();
    let version = app_version(app.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let accounts = accounts::load_slots().into_iter().map(|s| backup::Account { provider: s.provider, id: s.id, email: s.email });
        let now = chrono::Utc::now();
        let contents = backup::Contents::new(&settings, &history, &ledger, accounts, &version, now);
        let stamp = now.with_timezone(&chrono::Local).format("%Y-%m-%d-%H%M%S").to_string();
        let target = backup::target(path.as_deref(), csv_export::default_folder(), &stamp)?;
        let bytes = backup::seal(&contents, &passphrase)?;
        backup::write(&target, &bytes)?;
        Ok(serde_json::json!({ "path": target, "bytes": bytes.len(), "note": backup::NOTE }))
    }).await.map_err(|_| "backup_unwritable".to_string())?
}

/// Opens a backup and previews merging it; unless `dry_run`, also saves and applies the result.
async fn apply_backup(app: &AppHandle, bytes: Vec<u8>, passphrase: backup::Passphrase, dry_run: bool) -> Result<backup::Preview, String> {
    let contents = tauri::async_runtime::spawn_blocking(move || backup::open(&bytes, &passphrase))
        .await.map_err(|_| "backup_unreadable".to_string())??;
    let known: Vec<(Provider, String)> = tauri::async_runtime::spawn_blocking(|| accounts::load_slots().into_iter().map(|s| (s.provider, s.id)).collect())
        .await.unwrap_or_default();
    let state = app.state::<AppState>();
    let held = state.settings_lock.lock().await;
    let current = state.settings.lock().unwrap().clone();
    let now = chrono::Utc::now();
    let (merged_settings, preview, history, ledger) = {
        let folder = |f: &str| pins::normalize_folder(f).ok();
        let presence_ok = |f: &str| settings::valid_presence_folder(f);
        let calendar_ok = |p: &str| calendar::valid_path(p);
        let here = backup::Here { known: &known, updates_enabled: updates_enabled(), folder: &folder, presence_folder_ok: &presence_ok, calendar_path_ok: &calendar_ok };
        // Both held while merging, so a refresh finishing meanwhile is not lost.
        let mut history = state.usage_history.lock().unwrap();
        let mut ledger = state.ledger.lock().unwrap();
        let (merged, preview) = backup::merge(&current, &history, &ledger, &contents, &here, now);
        if dry_run { return Ok(preview); }
        *history = merged.history.clone();
        *ledger = merged.ledger.clone();
        (merged.settings, preview, merged.history, merged.ledger)
    };
    subscriptions::save(&history)?;
    ledger::save(&ledger)?;
    store_settings(app, merged_settings).await?;
    drop(held);
    // Accounts to sign in again here: "Sign in all accounts" walks through them by email.
    let mut pending = bulk::load_pending();
    bulk::add_pending(&mut pending, preview.accounts.iter().filter(|a| !a.connected)
        .map(|a| bulk::Pending { provider: a.provider, id: a.id.clone(), email: a.email.clone() }));
    let _ = bulk::save_pending(&pending);
    sync_pins(app, None).await;
    do_refresh(app).await;
    Ok(preview)
}

#[tauri::command]
async fn export_backup(app: AppHandle, path: Option<String>, passphrase: backup::Passphrase) -> Result<serde_json::Value, String> {
    write_backup(&app, path, passphrase).await
}

/// The Settings window reads the chosen file itself and passes its contents.
#[tauri::command]
async fn restore_backup(app: AppHandle, contents: String, passphrase: backup::Passphrase, dry_run: bool) -> Result<backup::Preview, String> {
    apply_backup(&app, contents.into_bytes(), passphrase, dry_run).await
}

// ---------------------------------------------------------------------------------------------
// Phone alerts (see `phone`)

#[tauri::command]
fn phone_token_saved() -> bool {
    settings::app_dir().is_some_and(|d| phone::load_token(&d).is_some())
}

/// Saves the access token with DPAPI; null or empty deletes it.
#[tauri::command]
async fn set_phone_token(token: Option<String>) -> Result<(), String> {
    let dir = settings::app_dir().ok_or("phone_token_storage")?;
    tauri::async_runtime::spawn_blocking(move || phone::store_token(&dir, token.as_deref())).await.map_err(|_| "phone_token_storage".to_string())?
}

// ---------------------------------------------------------------------------------------------
// Account roster (see `roster`)

/// Writes this machine's roster (with its "Sign in all accounts" run) when due and reads the
/// others' (at once with `force`, sooner with `soon`: see `roster::Syncer::sync`). Does nothing
/// without a sharing location or a passphrase.
async fn sync_roster(state: &AppState, settings: &settings::Settings, accounts: Vec<model::AccountUsage>, force: bool, soon: bool) {
    let (Some(store), Some(passphrase)) = (roster::store(settings), settings::app_dir().and_then(|d| roster::load_passphrase(&d))) else { return };
    let sync = state.roster.clone();
    let run = state.roster_run.lock().unwrap().clone();
    let now = chrono::Utc::now();
    let seen = tauri::async_runtime::spawn_blocking(move || {
        let (id, name) = presence::identity()?;
        let mut sync = sync.lock().unwrap();
        if let Some(run) = run {
            sync.offer_run(run);
        }
        sync.sync(store.as_ref(), &passphrase, (id, name), &accounts, now, force, soon);
        Some(sync.seen.clone())
    }).await.ok().flatten();
    if let Some(seen) = seen {
        *state.roster_seen.lock().unwrap() = seen;
    }
}

/// Sign-ins started through the door (often by an agent over SSH) open windows on this app's
/// desktop: refused while nobody can see it, so a sign-in never waits unseen (see `desktop`).
fn desktop_shown() -> Result<(), String> {
    match desktop::of_process().and_then(|d| d.problem()) {
        Some(problem) => Err(problem.into()),
        None => Ok(()),
    }
}

/// The door's `get_readiness`: whether sign-ins can run here, and what is waiting.
async fn readiness(app: &AppHandle) -> serde_json::Value {
    let roster = roster_status(app, false).await;
    let desktop = desktop::of_process();
    let browser = tauri::async_runtime::spawn_blocking(|| signin::find_browser().map(|(k, _)| pairing::browser_name(k))).await.ok().flatten();
    let (ready, mut problems, mut next) = pairing::assess(true, desktop, browser.is_some(), &roster);
    let state = app.state::<AppState>();
    // A remote named in settings shares nothing unless rclone here has it.
    let remote = state.settings.lock().unwrap().presence_remote.clone();
    if let Some(name) = remote {
        if !rclone_remotes().await.remotes.iter().any(|r| r.name == name) {
            problems.push("rclone_remote_missing");
            if ready {
                next = format!("Sharing names the rclone remote {name}, but rclone here is missing or lacks it, so no roster is written or read: install rclone and create that remote (rclone config) at this computer's screen.");
            }
        }
    }
    let snapshot = state.snapshot.lock().unwrap().clone();
    let signin_window = state.settings.lock().unwrap().signin_window;
    let mut summary = roster.clone();
    let waiting = summary.get_mut("waiting").map(serde_json::Value::take).unwrap_or_default();
    summary["waiting"] = waiting.as_array().map_or(0, Vec::len).into();
    serde_json::json!({
        "app": { "running": true, "version": app_version(app.clone()), "build": diagnostics::build_info(), "pid": std::process::id() },
        // The id its roster carries: the computer asking notes which SSH host reaches it.
        "machine": presence::identity().map(|(id, name)| serde_json::json!({ "id": id, "name": name })),
        "desktop": desktop, "idle_seconds": desktop::idle_seconds(), "browser": browser, "signin_window": signin_window,
        "accounts": { "here": snapshot.accounts.len(), "unmonitored": bulk::unmonitored(&snapshot).len() },
        "roster": summary, "waiting": waiting,
        // Sign in all accounts as it runs: email g of emails, done of total, who is signing in
        // now and for what (add or connect), why it stopped, and each step's outcome.
        "sign_in": state.bulk_connect.lock().unwrap().clone(),
        // Pass as `since` with `wait_seconds` to hear of the next change.
        "sign_in_revision": state.bulk_revision.load(std::sync::atomic::Ordering::SeqCst),
        "ready_for_sign_in": ready, "problems": problems, "next": next,
    })
}

/// Settings > Other computers and the door's `get_roster`. With `sync`, writes and reads the
/// rosters first.
async fn roster_status(app: &AppHandle, sync: bool) -> serde_json::Value {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let snapshot = state.snapshot.lock().unwrap().clone();
    if sync && settings.account_roster {
        sync_roster(&state, &settings, snapshot.accounts.clone(), true, false).await;
    }
    let seen = state.roster_seen.lock().unwrap().clone();
    let waiting = roster_waiting(&state, &snapshot);
    serde_json::json!({
        "enabled": settings.account_roster,
        "location": settings.presence_remote.is_some() || settings.presence_folder.is_some(),
        "passphrase_saved": roster_passphrase_saved(),
        "wrote": seen.wrote, "written_at": seen.written_at, "read": seen.read, "read_at": seen.read_at,
        "own_accounts": seen.own_accounts, "unreadable": seen.unreadable, "unsupported": seen.unsupported,
        "machines": seen.peers.iter().map(|p| serde_json::json!({ "machine": p.machine, "written_at": p.written_at, "accounts": p.accounts.len() })).collect::<Vec<_>>(),
        "waiting": waiting,
    })
}

#[tauri::command]
async fn get_roster(app: AppHandle, sync: Option<bool>) -> serde_json::Value {
    roster_status(&app, sync.unwrap_or(false)).await
}

/// Settings > Accounts' "Signing in on <machine>" cards: the other computers' runs of "Sign in all
/// accounts" as their rosters carry them (read sooner while Settings is `watching`; see
/// `roster::peer_runs`), and whether Retry can reach each one over SSH.
#[tauri::command]
async fn peer_sign_ins(app: AppHandle, watching: Option<bool>) -> Vec<serde_json::Value> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    if !settings.account_roster {
        return vec![];
    }
    let accounts = state.snapshot.lock().unwrap().accounts.clone();
    sync_roster(&state, &settings, accounts, false, watching.unwrap_or(false)).await;
    let dir = settings::app_dir();
    let runs = roster::peer_runs(&state.roster_seen.lock().unwrap(), chrono::Utc::now());
    runs.into_iter().map(|r| {
        let reachable = dir.as_deref().and_then(|d| pairing::host_for(d, &r.machine_id)).is_some();
        let mut card = serde_json::to_value(&r).unwrap_or_default();
        card["reachable"] = reachable.into();
        card
    }).collect()
}

/// Retry on a "Signing in on <machine>" card: runs Sign in all accounts on that computer again
/// through its SSH door (see `pairing::remote_call`), first stopping a run left waiting there. The
/// computer must have been paired or checked from here (`pairing::host_for`).
#[tauri::command]
async fn retry_peer_sign_in(app: AppHandle, machine_id: String) -> Result<(), String> {
    let host = settings::app_dir().and_then(|d| pairing::host_for(&d, &machine_id)).ok_or("peer_not_paired")?;
    let monitoring = app.state::<AppState>().roster_seen.lock().unwrap().peers.iter()
        .find(|p| p.machine_id == machine_id).and_then(|p| p.run.as_ref()).is_none_or(|r| r.monitoring);
    let arguments = serde_json::json!({ "confirm": true, "monitoring": monitoring, "retry": true });
    tauri::async_runtime::spawn_blocking(move || pairing::remote_call(&pairing::Ssh, &host, "sign_in_all", &arguments))
        .await.map_err(|_| "ssh_failed".to_string())??;
    // Its new run reaches the roster a few seconds later: read it at the live pace meanwhile.
    app.state::<AppState>().roster.lock().unwrap().expect_news(&machine_id, chrono::Utc::now());
    Ok(())
}

#[tauri::command]
fn roster_passphrase_saved() -> bool {
    settings::app_dir().is_some_and(|d| roster::load_passphrase(&d).is_some())
}

/// Saves the roster passphrase with DPAPI (null deletes it), then writes and reads the rosters
/// with it. A new passphrase forgets the keys derived from the old one.
async fn set_roster_passphrase(app: &AppHandle, passphrase: Option<backup::Passphrase>) -> Result<serde_json::Value, String> {
    let dir = settings::app_dir().ok_or("roster_passphrase_storage")?;
    tauri::async_runtime::spawn_blocking(move || roster::store_passphrase(&dir, passphrase.as_ref()))
        .await.map_err(|_| "roster_passphrase_storage".to_string())??;
    let state = app.state::<AppState>();
    state.roster.lock().unwrap().reset();
    *state.roster_seen.lock().unwrap() = roster::Seen::default();
    Ok(roster_status(app, true).await)
}

#[tauri::command]
async fn save_roster_passphrase(app: AppHandle, passphrase: Option<backup::Passphrase>) -> Result<serde_json::Value, String> {
    set_roster_passphrase(&app, passphrase).await
}

/// "Send test": the saved URL and token, whether or not phone alerts are on.
#[tauri::command]
async fn send_phone_test(app: AppHandle, title: String, body: String) -> Result<(), String> {
    let phone_settings = app.state::<AppState>().settings.lock().unwrap().phone.clone();
    let url = phone_settings.url.clone().ok_or("phone_url_missing")?;
    let message = phone::compose(&phone_settings, phone::Kind::Test, (&title, &body), (&title, &body)).ok_or("phone_url_missing")?;
    let token = settings::app_dir().and_then(|d| phone::load_token(&d));
    phone::send(&phone::client(), &url, token.as_deref(), &message).await
}

/// Re-reads the Codex CLI's local history off the async runtime, one scan at a time, then has
/// open windows reload their report. Its first run reads weeks of rollouts and can take a while.
fn scan_codex_local(app: &AppHandle) {
    static SCANNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if SCANNING.swap(true, std::sync::atomic::Ordering::AcqRel) { return; }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let local = tauri::async_runtime::spawn_blocking(|| codex_local::scan(chrono::Utc::now())).await;
        SCANNING.store(false, std::sync::atomic::Ordering::Release);
        let Ok(local) = local else { return };
        let state = app.state::<AppState>();
        *state.codex_local.lock().unwrap() = Some(std::sync::Arc::new(local));
        let snapshot = state.snapshot.lock().unwrap().clone();
        let _ = app.emit("usage-updated", &snapshot);
        crate::alerts::on_snapshot(&app);
    });
}

/// Changes the automatic-connect state, saving it when `change` says it changed.
fn auto_monitor_saved(app: &AppHandle, change: impl FnOnce(&mut auto_monitor::Tracker) -> bool) {
    let state = app.state::<AppState>();
    let saved = {
        let mut tracker = state.auto_monitor.lock().unwrap();
        change(&mut tracker).then(|| tracker.saved.clone())
    };
    if let Some(saved) = saved {
        let _ = auto_monitor::save(&saved);
    }
}

/// Starts monitoring's sign-in for the account in use when it is due (see `auto_monitor`). Reads
/// the last snapshot, so it is cheap to call often.
fn auto_connect_tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    // One sign-in at a time: never alongside an open one or a running "Connect monitoring for all".
    let queued = state.bulk_connect.lock().unwrap().as_ref().is_some_and(|b| !b.finished);
    if queued || state.account_operations.busy(monitoring::SIGN_IN) {
        return;
    }
    let enabled = state.settings.lock().unwrap().auto_connect_monitoring;
    let seen: Vec<_> = state.snapshot.lock().unwrap().accounts.iter().map(auto_monitor::Seen::of).collect();
    let fullscreen = enabled && dock::fullscreen();
    let busy = |key: &str| state.account_operations.busy(key);
    let guards = auto_monitor::Guards { enabled, fullscreen, busy: &busy };
    let decision = state.auto_monitor.lock().unwrap().decide(&seen, &guards, chrono::Utc::now());
    let auto_monitor::Decision::Start { provider, id } = decision else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let key = format!("{}:{id}", provider.name());
        if let Err(e) = start_monitoring_login(app.clone(), provider, id, true, None).await {
            // Someone else started on it meanwhile: no reason to wait a day.
            let ok = e == "monitor_busy";
            auto_monitor_saved(&app, |t| t.finished(&key, ok, chrono::Utc::now()));
        }
    });
}

/// Refreshes on the interval from settings, re-reading it so changes apply without a restart.
async fn refresh_loop(app: AppHandle) {
    loop {
        do_refresh(&app).await;
        auto_connect_tick(&app);
        let started = Instant::now();
        let mut seconds = 0u32;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            seconds += 1;
            // Every half minute, so an account is connected soon after its 3 minutes in use.
            if seconds % 30 == 0 { auto_connect_tick(&app); }
            let minutes = app.state::<AppState>().settings.lock().unwrap().refresh_minutes.max(1);
            if started.elapsed() >= Duration::from_secs(minutes as u64 * 60) {
                break;
            }
        }
    }
}

/// The app's context. An isolated run (`SLOPBAR_DATA_DIR`) takes an identifier of its own, so
/// its WebView2 data folder, single-instance mutex and notification id are its own too.
fn context() -> tauri::Context {
    let mut context = tauri::generate_context!();
    if settings::isolated() {
        let config = context.config_mut();
        config.identifier = format!("{}.isolated", config.identifier);
    }
    // Every window renders in software (see `additionalBrowserArgs` in tauri.conf.json);
    // SLOPBAR_GPU=1 brings the GPU process back, should a driver render something wrong.
    if std::env::var_os("SLOPBAR_GPU").is_some_and(|v| v == "1") {
        for w in context.config_mut().app.windows.iter_mut() {
            w.additional_browser_args = w.additional_browser_args.take().map(|a| a.replace(" --disable-gpu", ""));
        }
    }
    context
}

/// Whether an exit request may end the app. Tauri asks with no code when its last window closes,
/// which here only means the flyout was dismissed; an explicit exit (Quit, restart, update) carries one.
fn exit_allowed(code: Option<i32>) -> bool {
    code.is_some()
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;

    #[test]
    fn a_codex_sign_in_with_the_right_email_is_a_workspace_mismatch() {
        let (a, b) = (Some("synthetic.one@example.invalid"), Some("Synthetic.One@example.invalid "));
        assert_eq!(wrong_account(Provider::Codex, a, b), "monitor_wrong_workspace");
        assert_eq!(wrong_account(Provider::Codex, a, Some("synthetic.two@example.invalid")), "monitor_wrong_account");
        assert_eq!(wrong_account(Provider::Codex, None, b), "monitor_wrong_account");
        assert_eq!(wrong_account(Provider::Claude, a, b), "monitor_wrong_account", "a Claude organization is not a workspace choice");
    }

    #[test]
    fn a_run_reports_every_step() {
        let step = StepRecord { target: bulk::Target { provider: Provider::Codex, id: "synthetic-id".into(), account: Some("synthetic@example.invalid".into()), ended: None },
            kind: bulk::Kind::Connect, state: StepState::Failed, error: Some("signin_no_page".into()), started_at: None, page_at: None, ended_at: None };
        let v = serde_json::to_value(&step).unwrap();
        assert_eq!(v["provider"], "codex");
        assert_eq!(v["id"], "synthetic-id");
        assert_eq!(v["kind"], "connect");
        assert_eq!(v["state"], "failed");
        assert_eq!(v["error"], "signin_no_page");
        let done = StepRecord { state: StepState::Done, error: None, ..step.clone() };
        assert!(serde_json::to_value(&done).unwrap().get("error").is_none());
        // The same run as the roster carries it to the other computers.
        let bulk = BulkConnect { total: 2, done: 1, current: None, kind: None, emails: 1, email: 1, finished: true, error: Some("signin_no_page".into()),
            steps: vec![done, step], by_email: true, monitoring: true, still_waiting: false, revision: 9 };
        let run = roster_run(&bulk, "2026-10-23T12:00:00Z".parse().unwrap());
        assert_eq!((run.revision, run.finished, run.error.as_deref()), (9, true, Some("signin_no_page")));
        assert_eq!(run.steps.iter().map(|s| (s.kind.as_str(), s.state.as_str())).collect::<Vec<_>>(), [("connect", "done"), ("connect", "failed")]);
        assert_eq!(run.steps[1].account.as_deref(), Some("synthetic@example.invalid"));
    }

    #[test]
    fn retry_from_another_computer_is_opt_in() {
        let plain: crate::door::Request = serde_json::from_value(serde_json::json!({ "op": "sign_in_all", "confirm": true })).unwrap();
        assert!(matches!(plain, crate::door::Request::SignInAll { confirm: true, monitoring: true, retry: false }));
        let retry: crate::door::Request = serde_json::from_value(serde_json::json!({ "op": "sign_in_all", "confirm": true, "retry": true })).unwrap();
        assert!(matches!(retry, crate::door::Request::SignInAll { retry: true, .. }));
    }

    #[test]
    fn readiness_waits_only_when_asked_and_old_apps_still_read_it() {
        let plain = serde_json::to_value(crate::door::Request::GetReadiness { wait_seconds: None, since: None }).unwrap();
        assert_eq!(plain, serde_json::json!({ "op": "get_readiness" }), "no new fields for an app that does not know them");
        let waiting: crate::door::Request = serde_json::from_value(serde_json::json!({ "op": "get_readiness", "wait_seconds": 30, "since": 7 })).unwrap();
        assert!(matches!(waiting, crate::door::Request::GetReadiness { wait_seconds: Some(30), since: Some(7) }));
    }
}

#[cfg(test)]
mod exit_tests {
    #[test]
    fn closing_the_last_window_never_quits_but_an_explicit_exit_does() {
        assert!(!super::exit_allowed(None), "dismissing the flyout must not quit SlopBar");
        assert!(super::exit_allowed(Some(0)));
        assert!(super::exit_allowed(Some(1)));
    }
}

pub fn run() {
    migrate::run();
    let diagnostics = std::sync::Arc::new(diagnostics::Recorder::new(settings::app_dir().expect("config directory").join("diagnostics/history.bin")));
    let loaded = settings::load();
    // Off unless the user turned crash records on; nothing is recorded before the journal starts.
    diagnostics::set_crash_records(loaded.crash_records);
    diagnostics::install_crash_hook(diagnostics.clone());
    // Managed before the builder runs: config windows load (and call commands) before `setup`.
    let state = AppState {
        snapshot: Mutex::default(),
        settings: Mutex::new(loaded),
        menu: Mutex::default(),
        menu_items: Mutex::default(),
        panel_anchor: Mutex::default(),
        panel_height: Mutex::new(320.0),
        panel_hidden_at: Mutex::default(),
        dock_width: Mutex::new(160.0),
        usage_cache: Mutex::new(providers::load_cache()),
        usage_history: Mutex::new(subscriptions::load()),
        codex_local: Mutex::default(),
        active_log: Mutex::new(active_log::load()),
        active_saved: Mutex::default(),
        usage_index: Mutex::default(),
        monitor: monitoring::Monitor::new(settings::app_dir().expect("config directory").join("monitoring")).with_diagnostics(diagnostics.clone()),
        diagnostics: diagnostics.clone(),
        refresh_lock: tokio::sync::Mutex::new(()),
        settings_lock: tokio::sync::Mutex::new(()),
        account_operations: monitoring::Operations::default(),
        last_switch: Mutex::default(),
        wind_down: Mutex::default(),
        recent: Mutex::default(),
        sources: Mutex::default(),
        presence_tracks: Mutex::default(),
        roster: Default::default(),
        roster_seen: Mutex::default(),
        roster_run: Mutex::default(),
        unexpected_log: Mutex::new(unexpected::load()),
        plan_alerts: Mutex::new(plan_alerts::load()),
        burn_log: Mutex::new(burn::load()),
        burn_due: Mutex::default(),
        ledger: Mutex::new(ledger::load()),
        auto_monitor: Mutex::new(auto_monitor::Tracker::new(auto_monitor::load())),
        health: Mutex::default(),
        profile_states: Mutex::default(),
        bulk_connect: Mutex::default(),
        bulk_stop: Mutex::default(),
        bulk_changed: tokio::sync::Notify::new(),
        bulk_revision: Default::default(),
        hotkey_error: Mutex::default(),
        compact_hotkey_error: Mutex::default(),
        compact_item: Mutex::default(),
        settings_page: Mutex::default(),
        leases: Mutex::default(),
    };
    tauri::Builder::default()
        // Must come first: a second launch hands over to the running instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, shortcut, event| {
                if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                    hotkey_pressed(app, shortcut);
                }
            })
            .build())
        .manage(state)
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_usage,
            get_health,
            restart_app,
            get_subscriptions,
            get_sparklines,
            rescan_wsl,
            hide_panel,
            get_usage_breakdown,
            export_csv,
            csv_default_folder,
            export_backup,
            restore_backup,
            phone_token_saved,
            get_roster,
            peer_sign_ins,
            retry_peer_sign_in,
            roster_passphrase_saved,
            save_roster_passphrase,
            set_phone_token,
            send_phone_test,
            take_plan_alerts,
            take_capacity_alerts,
            get_credit_advice,
            take_digest,
            export_calendar,
            check_calendar_path,
            default_calendar_path,
            open_billing_page,
            export_diagnostics,
            record_frontend_error,
            refresh,
            get_settings,
            sync_folders,
            check_presence_folder,
            rclone_remotes,
            system_locale,
            check_for_updates,
            install_update,
            app_version,
            updates_enabled,
            save_settings,
            open_settings,
            take_settings_page,
            set_panel_height,
            set_menu_labels,
            switch_account,
            add_account,
            connect_monitoring,
            disconnect_monitoring,
            revoke_account_sessions,
            remove_account,
            connect_monitoring_all,
            sign_in_all_accounts,
            forget_signin_profiles,
            stop_sign_ins,
            retry_sign_ins,
            remove_lapsed_accounts,
            bulk_status,
            list_project_pins,
            set_project_pin,
            open_switcher,
            hide_switcher,
            hotkey_status,
            toggle_compact,
            set_dock_width,
            dock_clicked,
            dock_menu,
            accent_colors,
            notify::notify,
            notify::notify_unexpected,
            has_mica,
            taskbar_theme
        ])
        .setup(|app| {
            app.state::<AppState>().diagnostics.start();
            // Old diagnostics exports and window captures would otherwise pile up for ever.
            tauri::async_runtime::spawn_blocking(|| {
                if let Some(dir) = door::exports_dir() { diagnostics::prune_exports(&dir, std::time::SystemTime::now()); }
                if let Some(dir) = inspect::captures_dir() { inspect::prune(&dir); }
                // The browser profiles earlier versions kept per email (they offered stale Google
                // accounts), and fresh ones a SlopBar that ended mid-sign-in left behind.
                let _ = signin::cleanup();
            });
            // Before the first refresh reads the vault; a slot that fails stays readable as plaintext.
            let vault = accounts::migrate_vault();
            if !vault.is_empty() {
                let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
                app.state::<AppState>().diagnostics.record(None, None, diagnostics::Event::Vault {
                    migrated: count(vault.migrated), failed: count(vault.failed), unreadable: count(vault.unreadable),
                });
            }
            let handle = app.handle();
            #[cfg(windows)]
            {
                let registered = notify::register_app_id(&app.config().identifier);
                app.state::<AppState>().health.lock().unwrap().notifications_registered = Some(registered);
            }

            let defaults = ["Refresh", "Settings…", "Quit"];
            let items = MENU_IDS
                .iter()
                .zip(defaults)
                .map(|(id, text)| MenuItem::with_id(app, *id, text, true, None::<&str>))
                .collect::<Result<Vec<_>, _>>()?;
            let compact_on = app.state::<AppState>().settings.lock().unwrap().compact_flyout;
            let compact = CheckMenuItem::with_id(app, MENU_COMPACT, "Compact flyout", true, compact_on, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&items[0], &compact, &items[1], &separator, &items[2]])?;
            *app.state::<AppState>().menu.lock().unwrap() = Some(menu);
            *app.state::<AppState>().menu_items.lock().unwrap() = items;
            *app.state::<AppState>().compact_item.lock().unwrap() = Some(compact);
            app.on_menu_event(|app, event| menu_event(app, event.id().as_ref()));

            apply_settings(handle);
            apply_hotkey(handle);
            dock::show_docked(handle);
            // Built hidden now, so the first click on the widget opens the flyout at once.
            panel::prepare(handle);

            // Updates install silently; after one, say so once in the widget.
            let current = app.package_info().version.to_string();
            let previous = {
                let state = app.state::<AppState>();
                let mut s = state.settings.lock().unwrap();
                let previous = s.last_version.replace(current.clone());
                let _ = settings::save(&s);
                previous
            };
            if previous.is_some_and(|p| p != current) {
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    let _ = h.emit("update-installed", current);
                });
            }

            let h = handle.clone();
            std::thread::spawn(move || dock::keep_docked(h));
            tauri::async_runtime::spawn(refresh_loop(handle.clone()));
            if !settings::isolated() {
                tauri::async_runtime::spawn(updates::run_loop(handle.clone()));
            }
            // Agents use Settings' operations through `slopbar-mcp`, which knocks here.
            #[cfg(windows)]
            {
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    door::serve(door::pipe_name(), move |request| door_call(h.clone(), request)).await;
                });
            }
            Ok(())
        })
        .build(context())
        .expect("error while building slopbar")
        .run(|app, event| {
            // The taskbar widget is native and every web window is built on demand, so closing
            // the flyout can close the last Tauri window. Tauri then asks to exit (code None);
            // SlopBar lives in the taskbar, so only an explicit exit (Quit, restart, update) ends it.
            if let tauri::RunEvent::ExitRequested { code, api, .. } = &event {
                if !exit_allowed(*code) {
                    api.prevent_exit();
                }
                return;
            }
            // App::run exits the process directly: code after it cannot flush a journal.
            if matches!(event, tauri::RunEvent::Exit) {
                // Other machines stop showing this one's accounts as in use.
                let settings = app.state::<AppState>().settings.lock().unwrap().clone();
                if let Some(store) = presence::store(&settings) {
                    presence::withdraw(store.as_ref(), settings.refresh_minutes.max(1) as u64 * 60);
                }
                // A sign-in still open loses its browser and fresh profile with SlopBar.
                signin::close_all();
                app.state::<AppState>().diagnostics.record(None, None, diagnostics::Event::Shutdown);
            }
        });
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn windows() -> Vec<serde_json::Value> {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        config["app"]["windows"].as_array().unwrap().clone()
    }

    #[test]
    fn settings_and_the_switcher_are_created_only_when_opened() {
        for w in windows() {
            let label = w["label"].as_str().unwrap();
            // `create` defaults to true.
            let at_startup = w["create"].as_bool().unwrap_or(true);
            assert_eq!(at_startup, !ON_DEMAND.contains(&label), "{label}");
        }
        for label in ON_DEMAND {
            assert!(windows().iter().any(|w| w["label"] == label), "{label} has a configuration to be built from");
        }
    }

    #[test]
    fn every_window_shares_one_webview_environment() {
        // WebView2 refuses a webview whose browser arguments differ from the environment's, and
        // the same arguments keep every window in one browser, one GPU-less renderer per site.
        let args: Vec<_> = windows().iter().map(|w| w["additionalBrowserArgs"].as_str().map(String::from)).collect();
        assert!(args.iter().all(|a| a == &args[0]), "{args:?}");
        let args = args[0].clone().unwrap();
        for flag in ["--process-per-site", "--disable-gpu", "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"] {
            assert!(args.split(' ').any(|a| a == flag), "{flag} in {args}");
        }
        assert!(windows().iter().all(|w| w.get("dataDirectory").is_none()), "one WebView2 data folder for all");
    }
}
