use crate::{accounts, active_log, auto_monitor, backup, balancer, bulk, burn, calendar, codex_local, csv_export, diagnostics, digest, dock, door, headroom, health, inspect, leases, ledger, migrate, model, monitoring, notify, panel, phone, pins, plan_alerts, presence, providers, reset_credits, settings, sources, subscriptions, unexpected, updates, usage_index};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;

use accounts::Provider;
use model::Snapshot;
use settings::Settings;

pub(crate) const SETTINGS_LABEL: &str = "settings";
const MENU_IDS: [&str; 3] = ["refresh", "settings", "quit"];

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
    /// The past few hours of readings, for the agents' pace forecasts.
    pub(crate) recent: Mutex<headroom::Recent>,
    /// Sources found by the last refresh, where switches are written.
    pub(crate) sources: Mutex<Vec<sources::Source>>,
    /// Recent readings of unclaimed accounts, to notice use outside this PC.
    presence_tracks: Mutex<presence::Tracks>,
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
    /// Why the switcher's shortcut could not be registered (taken by another app, say).
    hotkey_error: Mutex<Option<String>>,
    /// The page Settings opens on when it is being created (it is destroyed when closed).
    settings_page: Mutex<Option<String>>,
    /// Accounts reserved for jobs (see `leases`); in memory only.
    pub(crate) leases: Mutex<leases::Leases>,
}

pub(crate) const SWITCHER_LABEL: &str = "switcher";

/// Progress of "Connect monitoring for all", sent to Settings as `bulk-monitoring`.
#[derive(Clone, Serialize)]
struct BulkConnect {
    total: usize,
    done: usize,
    /// The account whose sign-in is open now: sign in as this one.
    current: Option<bulk::Target>,
    finished: bool,
    error: Option<String>,
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
        R::AddAccount { provider } => {
            // One pending sign-in per provider, so a retrying agent cannot flood the desktop.
            let pending = app.state::<AppState>().account_operations.begin(provider, "add-account")?;
            start_add_account(app.clone(), provider, Some(pending)).await?;
            Ok(serde_json::json!({ "started": true, "next": "The user completes the sign-in in the terminal that opened; the account then appears in get_usage." }))
        }
        R::ConnectMonitoring { provider, id } => {
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
            let queued = connect_all(&app).await?;
            Ok(serde_json::json!({ "started": !queued.is_empty(), "queued": queued,
                "next": "Each sign-in opens after the previous one finishes; the user signs in as the account named in Settings. get_usage shows monitor_connected as they complete." }))
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
    // Probing for the CLI runs wsl.exe / where.exe; keep it off the async runtime.
    let job = tauri::async_runtime::spawn_blocking(move || accounts::start_login(provider, &found))
        .await
        .map_err(|e| e.to_string())??;
    tauri::async_runtime::spawn(async move {
        let _pending = pending;
        let added = job.wait(&http_client()).await;
        if let Some(slot) = added {
            let _ = app.emit("account-added", (slot.provider, slot.email));
            do_refresh(&app).await;
        }
    });
    Ok(())
}

/// A one-time login owns its own refresh-token chain; it never changes the active CLI.
#[tauri::command]
async fn connect_monitoring(app: AppHandle, provider: Provider, id: String) -> Result<(), String> {
    let key = format!("{}:{id}", provider.name());
    auto_monitor_saved(&app, |t| t.manual(&key, true));
    start_monitoring_login(app, provider, id, false).await.map(|_| ())
}

/// Connect monitoring's sign-in, started by the user or on its own (`auto`, see `auto_monitor`).
/// One sign-in is open at a time across the app (`monitoring::SIGN_IN`); the returned task
/// ends with its outcome once the user is done.
async fn start_monitoring_login(app: AppHandle, provider: Provider, id: String, auto: bool) -> Result<tauri::async_runtime::JoinHandle<Result<(), String>>, String> {
    let operation = app.state::<AppState>().account_operations.begin(provider, &id)?;
    let sign_in = app.state::<AppState>().account_operations.begin_key(monitoring::SIGN_IN)?;
    let expected = accounts::load_slots().into_iter().find(|s| s.provider == provider && s.id == id)
        .ok_or("monitor_wrong_account")?;
    let key = monitoring::key(&expected);
    let started = Instant::now();
    app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectStarted, code: diagnostics::Code::Ok, elapsed_ms: 0 });
    let found = app.state::<AppState>().sources.lock().unwrap().clone();
    let context = diagnostics::Context { recorder: app.state::<AppState>().diagnostics.clone(), provider, id: id.clone() };
    let job = tauri::async_runtime::spawn_blocking(move || accounts::start_login_observed(provider, &found, Some(context))).await;
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
                } else {
                    let mut known = slots;
                    known.push(expected.clone());
                    app.state::<AppState>().monitor.discard_unstored_login(&login, &known, &http_client()).await;
                    Err("monitor_wrong_account".into())
                }
            }
            None => Err("monitor_login_failed".into()),
        };
        drop(operation);
        drop(sign_in);
        if auto {
            let ok = outcome.is_ok();
            auto_monitor_saved(&app, |t| t.finished(&key, ok, chrono::Utc::now()));
        }
        app.state::<AppState>().diagnostics.record(Some(provider), Some(&id), diagnostics::Event::Action { action: diagnostics::Action::ConnectFinished,
            code: diagnostics::Code::from_error(outcome.as_ref().err().map(String::as_str)), elapsed_ms: diagnostics::millis(started) });
        let _ = app.emit("monitoring-login", serde_json::json!({ "provider": provider, "id": id, "error": outcome.as_ref().err() }));
        // Preserve last-known data, but allow the new login to bypass an earlier error's backoff.
        {
            let state = app.state::<AppState>();
            let _refresh = state.refresh_lock.lock().await;
            providers::retry_now(&mut state.usage_cache.lock().unwrap(), &key);
        }
        do_refresh(&app).await;
        outcome
    }))
}

/// Updates "Connect monitoring for all" and tells Settings.
fn publish_bulk(app: &AppHandle, change: impl FnOnce(&mut BulkConnect)) {
    let state = app.state::<AppState>();
    let mut running = state.bulk_connect.lock().unwrap();
    if let Some(bulk) = running.as_mut() {
        change(bulk);
        let _ = app.emit("bulk-monitoring", bulk.clone());
    }
}

/// Queues a monitoring sign-in for every account without one: one terminal at a time, each
/// started once the previous sign-in finished, stopping at the first that fails (the user closed
/// it, or signed in as another account). Returns the queued accounts.
async fn connect_all(app: &AppHandle) -> Result<Vec<bulk::Target>, String> {
    let state = app.state::<AppState>();
    let targets = bulk::unmonitored(&state.snapshot.lock().unwrap());
    {
        let mut running = state.bulk_connect.lock().unwrap();
        if running.as_ref().is_some_and(|b| !b.finished) {
            return Err("monitor_busy".into());
        }
        *running = Some(BulkConnect { total: targets.len(), done: 0, current: None, finished: targets.is_empty(), error: None });
    }
    let h = app.clone();
    let queue = targets.clone();
    tauri::async_runtime::spawn(async move {
        let mut error = None;
        for (done, target) in queue.iter().enumerate() {
            publish_bulk(&h, |b| { b.done = done; b.current = Some(target.clone()); });
            // Connected (or removed) since the queue started: nothing to do.
            let waiting = h.state::<AppState>().snapshot.lock().unwrap().accounts.iter()
                .any(|a| a.provider == target.provider && a.id == target.id && !a.monitor_connected);
            if !waiting {
                continue;
            }
            // Another sign-in is open (one the user or automatic monitoring started): wait for it.
            while h.state::<AppState>().account_operations.busy(monitoring::SIGN_IN) {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            // The user asked for these, so they count as manual starts, never automatic ones.
            let key = format!("{}:{}", target.provider.name(), target.id);
            auto_monitor_saved(&h, |t| t.manual(&key, true));
            let outcome = match start_monitoring_login(h.clone(), target.provider, target.id.clone(), false).await {
                Ok(task) => task.await.unwrap_or_else(|_| Err("monitor_login_failed".into())),
                Err(e) => Err(e),
            };
            if let Err(e) = outcome {
                error = Some(e);
                break;
            }
        }
        publish_bulk(&h, |b| {
            if error.is_none() { b.done = b.total; }
            b.current = None;
            b.finished = true;
            b.error = error;
        });
    });
    Ok(targets)
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
    })
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
            hide_switcher_window(&w);
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
    if let Some(panel) = app.get_webview_window(panel::LABEL) {
        let _ = panel.hide();
    }
    if let Some(w) = on_demand_window(&app, SWITCHER_LABEL).await {
        if !w.is_visible().unwrap_or(false) { show_switcher(&app, &w); }
    }
}

#[tauri::command]
fn hide_switcher(app: AppHandle) {
    if let Some(w) = app.get_webview_window(SWITCHER_LABEL) {
        hide_switcher_window(&w);
    }
}

/// Registers the switcher's system-wide shortcut from settings, replacing the previous one.
fn apply_hotkey(app: &AppHandle) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let hotkey = app.state::<AppState>().settings.lock().unwrap().switcher_hotkey.clone();
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    let error = if hotkey.is_empty() {
        None
    } else {
        match hotkey.replace("Win", "Super").parse::<Shortcut>() {
            Ok(shortcut) => shortcuts.register(shortcut).err().map(|_| "hotkey_taken".to_string()),
            Err(_) => Some("hotkey_invalid".to_string()),
        }
    };
    *app.state::<AppState>().hotkey_error.lock().unwrap() = error;
}

/// Why the shortcut is not working, if it is not: "hotkey_taken" or "hotkey_invalid".
#[tauri::command]
fn hotkey_status(state: tauri::State<AppState>) -> Option<String> {
    state.hotkey_error.lock().unwrap().clone()
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
    let stored = app.state::<AppState>().settings.lock().unwrap().switcher_hotkey.clone();
    let settings = if settings.switcher_hotkey == stored { settings } else {
        Settings { switcher_hotkey: settings::normalize_hotkey(&settings.switcher_hotkey)?, ..settings }
    };
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
    if previous.switcher_hotkey != settings.switcher_hotkey {
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
    if providers_changed || presence_changed {
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
        if let Err(e) = start_monitoring_login(app.clone(), provider, id, true).await {
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
        recent: Mutex::default(),
        sources: Mutex::default(),
        presence_tracks: Mutex::default(),
        unexpected_log: Mutex::new(unexpected::load()),
        plan_alerts: Mutex::new(plan_alerts::load()),
        burn_log: Mutex::new(burn::load()),
        burn_due: Mutex::default(),
        ledger: Mutex::new(ledger::load()),
        auto_monitor: Mutex::new(auto_monitor::Tracker::new(auto_monitor::load())),
        health: Mutex::default(),
        profile_states: Mutex::default(),
        bulk_connect: Mutex::default(),
        hotkey_error: Mutex::default(),
        settings_page: Mutex::default(),
        leases: Mutex::default(),
    };
    tauri::Builder::default()
        // Must come first: a second launch hands over to the running instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_settings(app)))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _shortcut, event| {
                if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                    toggle_switcher(app);
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
            remove_lapsed_accounts,
            bulk_status,
            list_project_pins,
            set_project_pin,
            open_switcher,
            hide_switcher,
            hotkey_status,
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
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&items[0], &items[1], &separator, &items[2]])?;
            *app.state::<AppState>().menu.lock().unwrap() = Some(menu);
            *app.state::<AppState>().menu_items.lock().unwrap() = items;
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
